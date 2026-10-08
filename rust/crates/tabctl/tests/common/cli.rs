use serde_json::Value;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

pub fn isolated_command(bin: &Path, config_home: &Path, state_home: &Path) -> Command {
    let mut command = Command::new(bin);
    for (key, _) in std::env::vars_os() {
        if key.to_string_lossy().starts_with("TABCTL_") {
            command.env_remove(key);
        }
    }
    command
        .env("HOME", config_home.parent().expect("sandbox config parent"))
        .env("XDG_CONFIG_HOME", config_home)
        .env("XDG_STATE_HOME", state_home)
        .env("TABCTL_AUTO_SYNC_MODE", "off");
    command
}

pub fn run_tabctl_output(
    bin: &Path,
    root: &Path,
    profile: &str,
    config_home: &Path,
    state_home: &Path,
    args: &[&str],
    timeout: Duration,
) -> Result<Output, String> {
    let mut child = isolated_command(bin, config_home, state_home)
        .args(["--json", "--no-pretty", "--profile", profile])
        .args(args)
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("Start tabctl {args:?}: {error}"))?;
    let mut stdout = child.stdout.take().expect("piped stdout");
    let mut stderr = child.stderr.take().expect("piped stderr");
    let out = thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout.read_to_end(&mut bytes).map(|_| bytes)
    });
    let err = thread::spawn(move || {
        let mut bytes = Vec::new();
        stderr.read_to_end(&mut bytes).map(|_| bytes)
    });
    let deadline = Instant::now() + timeout;
    let mut timed_out = false;
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        if Instant::now() >= deadline {
            timed_out = true;
            child.kill().map_err(|e| e.to_string())?;
            break child.wait().map_err(|e| e.to_string())?;
        }
        thread::sleep(Duration::from_millis(10));
    };
    let stdout = out
        .join()
        .map_err(|_| "stdout reader panicked")?
        .map_err(|e| e.to_string())?;
    let stderr = err
        .join()
        .map_err(|_| "stderr reader panicked")?
        .map_err(|e| e.to_string())?;
    if timed_out {
        return Err(format!(
            "tabctl {args:?} timed out after {timeout:?}: {}",
            String::from_utf8_lossy(&stderr)
        ));
    }
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

pub fn run_tabctl_json_with_timeout(
    bin: &Path,
    root: &Path,
    profile: &str,
    config_home: &Path,
    state_home: &Path,
    args: &[&str],
    timeout: Duration,
) -> Result<Value, String> {
    let output = run_tabctl_output(bin, root, profile, config_home, state_home, args, timeout)?;
    if !output.status.success() {
        return Err(format!(
            "tabctl {args:?} exited {}:\n{}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let parsed: Value = serde_json::from_slice(&output.stdout).map_err(|error| {
        format!(
            "Invalid CLI JSON: {error}: {}",
            String::from_utf8_lossy(&output.stdout)
        )
    })?;
    if parsed
        .get("errors")
        .and_then(Value::as_array)
        .is_some_and(|errors| !errors.is_empty())
    {
        return Err(format!("GraphQL operation failed: {parsed}"));
    }
    Ok(parsed)
}

pub fn run_tabctl_json(
    bin: &Path,
    root: &Path,
    profile: &str,
    config_home: &Path,
    state_home: &Path,
    args: &[&str],
) -> Result<Value, String> {
    run_tabctl_json_with_timeout(
        bin,
        root,
        profile,
        config_home,
        state_home,
        args,
        Duration::from_secs(30),
    )
}

pub fn run_tabctl_raw(bin: &Path, args: &[&str]) -> Result<(String, String), String> {
    let output = Command::new(bin)
        .args(args)
        .output()
        .map_err(|error| error.to_string())?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    if !output.status.success() {
        return Err(format!("tabctl {args:?} failed: {stderr}"));
    }
    Ok((stdout, stderr))
}
