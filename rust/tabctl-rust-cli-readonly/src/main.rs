use serde_json::{json, Value};
use std::env;
use std::io::{self, BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{self, Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Default)]
struct ParsedArgs {
    command: Option<String>,
    limit: Option<u64>,
    profile: Option<String>,
}

#[cfg(unix)]
fn default_socket_path() -> String {
    if let Ok(path) = env::var("TABCTL_SOCKET") {
        if !path.trim().is_empty() {
            return path;
        }
    }
    let state_base = env::var("XDG_STATE_HOME").ok().filter(|v| !v.trim().is_empty()).map(PathBuf::from).unwrap_or_else(|| {
        let home = env::var("HOME").unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join(".local").join("state")
    });
    state_base.join("tabctl").join("tabctl.sock").to_string_lossy().into_owned()
}

#[cfg(windows)]
fn default_socket_path() -> String {
    env::var("TABCTL_SOCKET").unwrap_or_else(|_| r"\\.\pipe\tabctl-rust-readonly".to_string())
}

fn parse_args(args: &[String]) -> Result<ParsedArgs, String> {
    let mut parsed = ParsedArgs::default();
    let mut i = 0usize;

    while i < args.len() {
        let arg = &args[i];
        if !arg.starts_with("--") {
            if parsed.command.is_none() {
                parsed.command = Some(arg.clone());
                i += 1;
                continue;
            }
            i += 1;
            continue;
        }

        match arg.as_str() {
            "--json" | "--pretty" | "--help" => {
                i += 1;
            }
            "--profile" => {
                let value = args.get(i + 1).ok_or_else(|| "Missing value for --profile".to_string())?;
                parsed.profile = Some(value.clone());
                i += 2;
            }
            "--limit" => {
                let value = args.get(i + 1).ok_or_else(|| "Missing value for --limit".to_string())?;
                let limit = value
                    .parse::<u64>()
                    .map_err(|_| "Invalid --limit value (must be a positive integer)".to_string())?;
                parsed.limit = Some(limit);
                i += 2;
            }
            _ => {
                i += 1;
            }
        }
    }

    Ok(parsed)
}

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0)
}

fn create_request_id() -> String {
    format!("req-{}-{}", now_millis(), process::id())
}

fn env_bool(name: &str) -> bool {
    let raw = env::var(name).unwrap_or_default();
    matches!(raw.trim().to_lowercase().as_str(), "1" | "true" | "yes")
}

fn env_value(name: &str, fallback: &str) -> String {
    let value = env::var(name).unwrap_or_default();
    let trimmed = value.trim();
    if trimmed.is_empty() {
        fallback.to_string()
    } else {
        trimmed.to_string()
    }
}

fn env_optional(name: &str) -> Option<String> {
    let value = env::var(name).unwrap_or_default();
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn print_json(payload: &Value) {
    match serde_json::to_string_pretty(payload) {
        Ok(text) => {
            let _ = writeln!(io::stdout(), "{}", text);
        }
        Err(err) => {
            let fallback = json!({
                "ok": false,
                "error": { "message": format!("Failed to serialize JSON: {}", err) }
            });
            let _ = writeln!(io::stdout(), "{}", fallback);
        }
    }
}

fn print_error(message: &str, hint: Option<&str>) -> ! {
    let mut payload = json!({
        "ok": false,
        "error": {
            "message": message
        }
    });
    if let Some(hint_text) = hint {
        payload["error"]["hint"] = Value::String(hint_text.to_string());
    }
    print_json(&payload);
    process::exit(1);
}

fn client_metadata() -> Value {
    let version = env_value("TABCTL_VERSION", env!("CARGO_PKG_VERSION"));
    let base_version = env_value("TABCTL_BASE_VERSION", &version);
    let git_sha = env_optional("TABCTL_GIT_SHA");
    let dirty = env_bool("TABCTL_DIRTY");
    json!({
        "component": "cli",
        "version": version,
        "baseVersion": base_version,
        "gitSha": git_sha,
        "dirty": dirty
    })
}

fn local_version_response() -> Value {
    let version = env_value("TABCTL_VERSION", env!("CARGO_PKG_VERSION"));
    let base_version = env_value("TABCTL_BASE_VERSION", &version);
    let git_sha = env_optional("TABCTL_GIT_SHA");
    let dirty = env_bool("TABCTL_DIRTY");
    json!({
        "ok": true,
        "data": {
            "version": version,
            "baseVersion": base_version,
            "gitSha": git_sha,
            "dirty": dirty,
            "component": "cli"
        }
    })
}

fn delegate_to_node_cli(args: &[String]) -> ! {
    let node_exec = env_optional("TABCTL_NODE_EXEC").unwrap_or_else(|| "node".to_string());
    let cli_bin = env_optional("TABCTL_NODE_CLI_BIN").unwrap_or_else(|| {
        print_error(
            "Command passthrough requires TABCTL_NODE_CLI_BIN when TABCTL_CLI_IMPL=rust",
            Some("Run via the Node tabctl entrypoint or set TABCTL_NODE_CLI_BIN"),
        )
    });

    let mut command = Command::new(node_exec);
    command.arg(cli_bin);
    command.args(args);
    command.env("TABCTL_CLI_IMPL", "node");
    command.stdin(Stdio::inherit());
    command.stdout(Stdio::inherit());
    command.stderr(Stdio::inherit());

    let status = command
        .status()
        .unwrap_or_else(|err| print_error(&format!("Failed to execute Node CLI fallback: {}", err), None));
    process::exit(status.code().unwrap_or(1));
}

#[cfg(unix)]
fn send_request(action: &str, params: Value) -> Result<Value, io::Error> {
    use std::os::unix::net::UnixStream;

    let socket_path = default_socket_path();
    let mut stream = UnixStream::connect(socket_path)?;
    let request = json!({
        "id": create_request_id(),
        "action": action,
        "params": params,
        "client": client_metadata()
    });
    let request_text = serde_json::to_string(&request)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, format!("Invalid request JSON: {}", err)))?;
    stream.write_all(request_text.as_bytes())?;
    stream.write_all(b"\n")?;

    let mut reader = BufReader::new(stream);
    loop {
        let mut line = String::new();
        let read = reader.read_line(&mut line)?;
        if read == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "No response received"));
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let response: Value = serde_json::from_str(trimmed)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, format!("Invalid response JSON: {}", err)))?;
        if response.get("progress").and_then(Value::as_bool).unwrap_or(false) {
            continue;
        }
        return Ok(response);
    }
}

#[cfg(windows)]
fn send_request(_action: &str, _params: Value) -> Result<Value, io::Error> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Windows named pipes are not implemented in rust cli readonly MVP",
    ))
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    let parsed = parse_args(&args).unwrap_or_else(|message| print_error(&message, None));

    if let Some(profile) = parsed.profile {
        env::set_var("TABCTL_PROFILE", profile);
    }

    let command = parsed
        .command
        .as_deref()
        .unwrap_or_else(|| print_error("Missing command", Some("Supported commands: version, ping, history")));

    if command == "version" {
        print_json(&local_version_response());
        return;
    }

    if command != "ping" && command != "history" {
        delegate_to_node_cli(&args);
    }

    let params = if command == "history" {
        match parsed.limit {
            Some(limit) => json!({ "limit": limit }),
            None => json!({}),
        }
    } else {
        json!({})
    };

    let response = send_request(command, params);

    match response {
        Ok(payload) => {
            print_json(&payload);
            let ok = payload.get("ok").and_then(Value::as_bool).unwrap_or(false);
            if !ok {
                process::exit(1);
            }
        }
        Err(err) => {
            let message = format!("Failed to connect to host: {}", err);
            let hint = if message.contains("No such file") || message.contains("ENOENT") {
                Some("Native host not running. Ensure the browser extension is loaded and active. If you recently upgraded, run: tabctl setup")
            } else {
                None
            };
            print_error(&message, hint);
        }
    }
}
