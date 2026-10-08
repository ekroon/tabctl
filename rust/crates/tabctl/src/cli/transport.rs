use super::*;
use std::io::ErrorKind;
use std::thread::sleep;
use std::time::{Duration, Instant};

const CONNECT_RETRY_TIMEOUT_MS: u64 = 3_000;
const CONNECT_RETRY_DELAY_MS: u64 = 100;

fn response_timeout_ms() -> u64 {
    std::env::var("TABCTL_RESPONSE_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(CLI_RESPONSE_TIMEOUT_MS)
}

pub(super) fn resolve_socket_endpoint(profile: Option<&str>) -> Result<SocketEndpoint, String> {
    if let Ok(transport) = std::env::var("TABCTL_TRANSPORT") {
        if !transport.trim().is_empty() && !matches!(transport.trim(), "socket" | "unix") {
            return Err("tabctl supports Unix-domain sockets only; remove TABCTL_TRANSPORT or set it to unix".to_string());
        }
    }
    if let Ok(path) = std::env::var("TABCTL_SOCKET") {
        if !path.trim().is_empty() {
            return SocketEndpoint::parse(&path);
        }
    }
    let data_dir = resolve_data_dir(profile)?;
    SocketEndpoint::parse(&path_to_platform_string(
        &PathBuf::from(&data_dir).join("tabctl.sock"),
    ))
}

pub(super) fn resolve_data_dir(profile: Option<&str>) -> Result<String, String> {
    if let Ok(path) = std::env::var("TABCTL_DATA_DIR") {
        if !path.trim().is_empty() {
            return Ok(normalize_path_for_current_platform(&path));
        }
    }
    let config_dir = resolve_config_dir()?;
    let profiles_path = PathBuf::from(&config_dir).join("profiles.json");
    let registry = fs::read_to_string(&profiles_path)
        .ok()
        .and_then(|contents| serde_json::from_str::<ProfileRegistry>(&contents).ok());
    if let Some(profile_name) = profile {
        if let Some(profile_entry) = registry
            .as_ref()
            .and_then(|registry| registry.profiles.get(profile_name))
        {
            return Ok(normalize_path_for_current_platform(&profile_entry.data_dir));
        }
        return Err(format!(
            "Profile \"{profile_name}\" not found in profiles.json"
        ));
    }
    if let Ok(path) = std::env::var("TABCTL_STATE_DIR") {
        if !path.trim().is_empty() {
            return Ok(normalize_path_for_current_platform(&path));
        }
    }
    if let Ok(path) = std::env::var("XDG_STATE_HOME") {
        return Ok(path_to_platform_string(&PathBuf::from(path).join("tabctl")));
    }
    let home = dirs::home_dir().ok_or_else(|| "Unable to resolve home directory".to_string())?;
    Ok(path_to_platform_string(
        &home.join(".local").join("state").join("tabctl"),
    ))
}

pub(super) fn resolve_config_dir() -> Result<String, String> {
    if let Ok(path) = std::env::var("TABCTL_CONFIG_DIR") {
        return Ok(normalize_path_for_current_platform(&path));
    }
    if let Ok(path) = std::env::var("XDG_CONFIG_HOME") {
        return Ok(path_to_platform_string(&PathBuf::from(path).join("tabctl")));
    }
    let home = dirs::home_dir().ok_or_else(|| "Unable to resolve home directory".to_string())?;
    Ok(path_to_platform_string(
        &home.join(".config").join("tabctl"),
    ))
}

pub(super) fn resolve_effective_profile(profile: Option<&str>) -> Option<String> {
    if let Some(name) = profile {
        return Some(name.to_string());
    }
    let config_dir = resolve_config_dir().ok()?;
    let profiles_path = PathBuf::from(config_dir).join("profiles.json");
    let contents = fs::read_to_string(profiles_path).ok()?;
    let registry = serde_json::from_str::<ProfileRegistry>(&contents).ok()?;
    registry
        .default
        .or_else(|| registry.profiles.keys().next().cloned())
}

pub(super) fn request_id() -> String {
    format!("req-{}-{}", now_ms(), std::process::id())
}

pub(super) fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

fn connect_with_retry<T>(mut connect: impl FnMut() -> std::io::Result<T>) -> std::io::Result<T> {
    let deadline = Instant::now() + Duration::from_millis(CONNECT_RETRY_TIMEOUT_MS);
    loop {
        match connect() {
            Ok(stream) => return Ok(stream),
            Err(err) if is_transient_connect_error(&err) && Instant::now() < deadline => {
                sleep(Duration::from_millis(CONNECT_RETRY_DELAY_MS));
            }
            Err(err) => return Err(err),
        }
    }
}

fn is_transient_connect_error(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        ErrorKind::ConnectionRefused | ErrorKind::NotFound | ErrorKind::WouldBlock
    )
}

pub(super) fn send_request(
    action: &str,
    params: Value,
    profile: Option<&str>,
    show_progress: bool,
) -> Result<ResponseEnvelope, String> {
    let effective_profile = resolve_effective_profile(profile);
    let SocketEndpoint::Unix { path } = resolve_socket_endpoint(effective_profile.as_deref())?;
    let stream = connect_with_retry(|| UnixStream::connect(path.as_str()))
        .map_err(|e| format!("Failed to connect to host: {e}"))?;
    send_request_over_stream(stream, action, params, show_progress)
}

pub(super) fn send_request_over_stream<S>(
    mut stream: S,
    action: &str,
    params: Value,
    show_progress: bool,
) -> Result<ResponseEnvelope, String>
where
    S: std::io::Read + Write + Send + 'static,
{
    let request = RequestEnvelope {
        id: Some(request_id()),
        action: action.to_string(),
        params,
        auth_token: None,
    };
    serde_json::to_writer(&mut stream, &request)
        .map_err(|e| format!("Failed to encode request: {e}"))?;
    stream
        .write_all(b"\n")
        .map_err(|e| format!("Failed to send request: {e}"))?;
    stream
        .flush()
        .map_err(|e| format!("Failed to flush request: {e}"))?;

    let timeout_ms = response_timeout_ms();
    let (tx, rx) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let reader = BufReader::new(stream);
        for line in reader.lines() {
            let line = match line {
                Ok(line) => line,
                Err(e) => {
                    let _ = tx.send(Err(format!("Failed to read response: {e}")));
                    return;
                }
            };
            if line.trim().is_empty() {
                continue;
            }
            let response: ResponseEnvelope = match serde_json::from_str(&line) {
                Ok(response) => response,
                Err(e) => {
                    let _ = tx.send(Err(format!("Invalid response payload: {e}")));
                    return;
                }
            };
            if response.progress.unwrap_or(false) {
                if show_progress {
                    let data = response.data.unwrap_or(json!({}));
                    eprintln!("[tabctl] progress: {}", data);
                }
                continue;
            }
            let _ = tx.send(Ok(response));
            return;
        }
        let _ = tx.send(Err("No response received".to_string()));
    });

    match rx.recv_timeout(Duration::from_millis(timeout_ms)) {
        Ok(result) => result,
        Err(mpsc::RecvTimeoutError::Timeout) => {
            Err(format!("Request timed out after {timeout_ms}ms"))
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            Err("Response reader disconnected unexpectedly".to_string())
        }
    }
}
