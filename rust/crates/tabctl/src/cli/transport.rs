use super::*;

#[cfg(windows)]
pub(super) fn format_windows_pipe_connect_error(
    profile: Option<&str>,
    data_dir: &str,
    pipe_path: &str,
    err: &std::io::Error,
) -> String {
    let profile_name = profile.unwrap_or("<default>");
    let code = err
        .raw_os_error()
        .map(|value| value.to_string())
        .unwrap_or_else(|| "unknown".to_string());
    let cause_hint = match err.raw_os_error() {
        Some(2) | Some(3) => {
            "The named pipe was not found. The host may not be running yet, or the CLI and wrapper may be resolving different profile/data-dir values."
        }
        Some(5) => {
            "Windows denied access to the named pipe. If the wrapper and CLI are running in the same shell/user context, the remaining likely causes are named-pipe ACLs or a profile/data-dir mismatch."
        }
        _ => {
            "Verify that the generated wrapper and the CLI resolve the same TABCTL_PROFILE, TABCTL_CONFIG_DIR, and TABCTL_DATA_DIR values."
        }
    };

    format!(
        "Failed to connect to host at named pipe {pipe_path}: {err} (os error {code})\nprofile: {profile_name}\ndata dir: {data_dir}\nhint: {cause_hint}"
    )
}

fn response_timeout_ms() -> u64 {
    std::env::var("TABCTL_RESPONSE_TIMEOUT_MS")
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(CLI_RESPONSE_TIMEOUT_MS)
}

pub(super) fn resolve_socket_endpoint(profile: Option<&str>) -> Result<SocketEndpoint, String> {
    if let Ok(path) = std::env::var("TABCTL_SOCKET") {
        if !path.trim().is_empty() {
            let endpoint = SocketEndpoint::parse(&path)?;
            #[cfg(target_os = "linux")]
            if is_wsl_environment() && matches!(endpoint, SocketEndpoint::Tcp { .. }) {
                return Err(
                    "TCP transport is disabled in WSL; use the Windows named-pipe bridge."
                        .to_string(),
                );
            }
            return Ok(endpoint);
        }
    }
    // Check TABCTL_TRANSPORT=tcp for explicit TCP transport selection
    if let Ok(transport) = std::env::var("TABCTL_TRANSPORT") {
        if transport.trim().eq_ignore_ascii_case("tcp") {
            #[cfg(target_os = "linux")]
            if is_wsl_environment() {
                return Err(
                    "TCP transport is disabled in WSL; use the Windows named-pipe bridge."
                        .to_string(),
                );
            }
            if let Ok(port_str) = std::env::var("TABCTL_TCP_PORT") {
                if let Ok(port) = port_str.trim().parse::<u16>() {
                    if port > 0 {
                        return Ok(SocketEndpoint::Tcp {
                            host: "127.0.0.1".to_string(),
                            port,
                        });
                    }
                }
            }
            let data_dir = resolve_data_dir(profile)?;
            let port_path = PathBuf::from(&data_dir).join(WSL_TCP_PORT_FILENAME);
            if let Some(port) = read_tcp_port_file(&port_path) {
                return Ok(SocketEndpoint::Tcp {
                    host: "127.0.0.1".to_string(),
                    port,
                });
            }
            return Err("TABCTL_TRANSPORT=tcp but no tcp-port file found. Is the host running with TABCTL_HOST_TCP=1?".to_string());
        }
    }
    #[cfg(target_os = "linux")]
    if is_wsl_environment() {
        return discover_wsl_pipe_endpoint(profile).ok_or_else(|| {
            "No WSL named-pipe endpoint was found. Is the Windows host running and publishing pipe-endpoint?"
                .to_string()
        });
    }
    let data_dir = resolve_data_dir(profile)?;
    #[cfg(windows)]
    {
        Ok(resolve_windows_pipe_endpoint(&data_dir))
    }
    #[cfg(not(windows))]
    {
        SocketEndpoint::parse(&path_to_platform_string(
            &PathBuf::from(&data_dir).join("tabctl.sock"),
        ))
    }
}

#[cfg(target_os = "linux")]
pub(super) fn is_wsl_environment() -> bool {
    std::env::var_os("WSL_INTEROP").is_some()
        || std::env::var_os("WSL_DISTRO_NAME").is_some()
        || fs::read_to_string("/proc/version")
            .map(|content| content.to_ascii_lowercase().contains("microsoft"))
            .unwrap_or(false)
}

pub(super) fn can_repair_host_wrapper() -> bool {
    #[cfg(target_os = "linux")]
    {
        !is_wsl_environment()
    }
    #[cfg(not(target_os = "linux"))]
    {
        true
    }
}

#[cfg(all(target_os = "linux", test))]
pub(super) fn discover_wsl_tcp_port_from_data_dir(data_dir: &str) -> Option<u16> {
    for path in wsl_tcp_port_candidates(data_dir) {
        if let Some(port) = read_tcp_port_file(&path) {
            return Some(port);
        }
    }
    None
}

#[cfg(target_os = "linux")]
pub(super) fn discover_wsl_pipe_endpoint(profile: Option<&str>) -> Option<SocketEndpoint> {
    let data_dir = resolve_data_dir(profile).ok()?;
    for path in wsl_pipe_endpoint_candidates(&data_dir) {
        if let Ok(content) = fs::read_to_string(path) {
            if let Ok(endpoint) = SocketEndpoint::parse(content.trim()) {
                if matches!(endpoint, SocketEndpoint::Pipe { .. }) {
                    return Some(endpoint);
                }
            }
        }
    }
    None
}

#[cfg(any(target_os = "linux", test))]
pub(super) fn resolve_windows_username_from_path() -> Option<String> {
    let path_env = std::env::var("PATH").ok()?;
    for entry in path_env.split(':') {
        let lower = entry.to_lowercase();
        if let Some(pos) = lower.find("/mnt/c/users/") {
            let after_prefix = &entry[pos + "/mnt/c/Users/".len()..];
            let username = after_prefix.split('/').next()?;
            if !username.is_empty() {
                return Some(username.to_string());
            }
        }
    }
    None
}

#[cfg(any(target_os = "linux", test))]
pub(super) fn resolve_windows_appdata_local() -> Option<PathBuf> {
    let username = resolve_windows_username_from_path()?;
    Some(PathBuf::from(format!(
        "/mnt/c/Users/{username}/AppData/Local"
    )))
}

#[cfg(target_os = "linux")]
pub(super) fn resolve_windows_appdata_roaming() -> Option<PathBuf> {
    let username = resolve_windows_username_from_path()?;
    Some(PathBuf::from(format!(
        "/mnt/c/Users/{username}/AppData/Roaming"
    )))
}

#[cfg(any(target_os = "linux", test))]
pub(super) fn wsl_file_candidates(data_dir: &str, filename: &str) -> Vec<PathBuf> {
    let mut candidates = vec![PathBuf::from(data_dir).join(filename)];
    if let Some(wsl_path) = windows_path_to_wsl_path(data_dir) {
        candidates.push(wsl_path.join(filename));
    }
    let Some(relative_suffix) = tabctl_relative_suffix(Path::new(data_dir)) else {
        return candidates;
    };
    let Some(appdata_local) = resolve_windows_appdata_local() else {
        return candidates;
    };
    candidates.push(
        appdata_local
            .join("tabctl")
            .join(&relative_suffix)
            .join(filename),
    );
    candidates.push(
        appdata_local
            .join("tabctl-state")
            .join("tabctl")
            .join(&relative_suffix)
            .join(filename),
    );
    candidates
}

#[cfg(any(target_os = "linux", test))]
pub(super) fn windows_path_to_wsl_path(path: &str) -> Option<PathBuf> {
    let normalized = path.replace('/', "\\");
    let bytes = normalized.as_bytes();
    if bytes.len() < 3 || bytes[1] != b':' || bytes[2] != b'\\' {
        return None;
    }
    let drive = (bytes[0] as char).to_ascii_lowercase();
    let suffix = normalized[3..].replace('\\', "/");
    Some(PathBuf::from(format!("/mnt/{drive}/{suffix}")))
}

#[cfg(all(target_os = "linux", test))]
pub(super) fn wsl_tcp_port_candidates(data_dir: &str) -> Vec<PathBuf> {
    wsl_file_candidates(data_dir, WSL_TCP_PORT_FILENAME)
}

#[cfg(target_os = "linux")]
pub(super) fn wsl_pipe_endpoint_candidates(data_dir: &str) -> Vec<PathBuf> {
    wsl_file_candidates(data_dir, WSL_PIPE_ENDPOINT_FILENAME)
}

#[cfg(any(target_os = "linux", test))]
pub(super) fn tabctl_relative_suffix(path: &Path) -> Option<PathBuf> {
    let mut relative = PathBuf::new();
    let mut found = false;
    for component in path.components() {
        if found {
            relative.push(component.as_os_str());
            continue;
        }
        if component.as_os_str() == "tabctl" {
            found = true;
        }
    }
    found.then_some(relative)
}

pub(super) fn read_tcp_port_file(path: &Path) -> Option<u16> {
    let content = fs::read_to_string(path).ok()?;
    let port = content.trim().parse::<u16>().ok()?;
    (port > 0).then_some(port)
}

#[cfg(windows)]
pub(super) fn resolve_windows_pipe_endpoint(data_dir: &str) -> SocketEndpoint {
    SocketEndpoint::Pipe {
        path: windows_pipe_path(data_dir),
    }
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
    #[cfg(windows)]
    if let Ok(path) = std::env::var("LOCALAPPDATA") {
        if !path.trim().is_empty() {
            return Ok(path_to_platform_string(&PathBuf::from(path).join("tabctl")));
        }
    }
    #[cfg(target_os = "linux")]
    if is_wsl_environment() {
        if let Some(appdata_local) = resolve_windows_appdata_local() {
            return Ok(path_to_platform_string(&appdata_local.join("tabctl")));
        }
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
    #[cfg(windows)]
    if let Ok(path) = std::env::var("APPDATA") {
        if !path.trim().is_empty() {
            return Ok(path_to_platform_string(&PathBuf::from(path).join("tabctl")));
        }
    }
    #[cfg(target_os = "linux")]
    if is_wsl_environment() {
        if let Some(appdata_roaming) = resolve_windows_appdata_roaming() {
            return Ok(path_to_platform_string(&appdata_roaming.join("tabctl")));
        }
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

pub(super) fn read_auth_token(profile: Option<&str>) -> Option<String> {
    if let Ok(token) = std::env::var("TABCTL_AUTH_TOKEN") {
        let token = token.trim().to_string();
        if !token.is_empty() {
            return Some(token);
        }
    }

    let data_dir = resolve_data_dir(profile).ok()?;

    #[cfg(target_os = "linux")]
    if is_wsl_environment() {
        for path in wsl_file_candidates(&data_dir, AUTH_TOKEN_FILENAME) {
            if let Ok(content) = fs::read_to_string(&path) {
                let token = content.trim().to_string();
                if !token.is_empty() {
                    return Some(token);
                }
            }
        }
        return None;
    }

    let path = PathBuf::from(&data_dir).join(AUTH_TOKEN_FILENAME);
    fs::read_to_string(&path).ok().and_then(|content| {
        let token = content.trim().to_string();
        if token.is_empty() {
            None
        } else {
            Some(token)
        }
    })
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

pub(super) fn send_request(
    action: &str,
    params: Value,
    profile: Option<&str>,
    show_progress: bool,
) -> Result<ResponseEnvelope, String> {
    let effective_profile = resolve_effective_profile(profile);
    let resolved_profile = effective_profile.as_deref();
    let endpoint = resolve_socket_endpoint(resolved_profile)?;
    match endpoint {
        SocketEndpoint::Unix { path } => {
            #[cfg(unix)]
            {
                let stream = UnixStream::connect(path)
                    .map_err(|e| format!("Failed to connect to host: {e}"))?;
                send_request_over_stream(stream, action, params, show_progress, None)
            }
            #[cfg(not(unix))]
            {
                let _ = path;
                Err("Unix socket transport is unsupported on this target".to_string())
            }
        }
        SocketEndpoint::Tcp { host, port } => {
            let auth_token = read_auth_token(resolved_profile);
            let stream = TcpStream::connect((host.as_str(), port))
                .map_err(|e| format!("Failed to connect to host: {e}"))?;
            send_request_over_stream(stream, action, params, show_progress, auth_token)
        }
        SocketEndpoint::Pipe { path } => {
            #[cfg(windows)]
            {
                let data_dir =
                    resolve_data_dir(resolved_profile).unwrap_or_else(|_| "<unknown>".to_string());
                let stream = fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(&path)
                    .map_err(|e| {
                        format_windows_pipe_connect_error(resolved_profile, &data_dir, &path, &e)
                    })?;
                send_request_over_stream(stream, action, params, show_progress, None)
            }
            #[cfg(target_os = "linux")]
            {
                if is_wsl_environment() {
                    return send_request_over_wsl_named_pipe(&path, action, params, show_progress);
                }
                Err("Named pipe transport is unsupported on this target".to_string())
            }
            #[cfg(not(any(windows, target_os = "linux")))]
            {
                let _ = path;
                Err("Named pipe transport is unsupported on this target".to_string())
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn wsl_named_pipe_name(pipe_path: &str) -> Result<String, String> {
    let trimmed = pipe_path.trim();
    if let Some(name) = trimmed.strip_prefix("pipe://") {
        let name = name.trim_start_matches('/');
        if !name.is_empty() {
            return Ok(name.to_string());
        }
    }
    if let Some(name) = trimmed.strip_prefix(r"\\.\pipe\") {
        if !name.is_empty() {
            return Ok(name.to_string());
        }
    }
    Err(format!(
        "Unsupported Windows pipe endpoint for WSL bridge: {pipe_path}"
    ))
}

#[cfg(target_os = "linux")]
fn parse_response_lines(output: &str, show_progress: bool) -> Result<ResponseEnvelope, String> {
    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let response: ResponseEnvelope =
            serde_json::from_str(trimmed).map_err(|e| format!("Invalid response payload: {e}"))?;
        if response.progress.unwrap_or(false) {
            if show_progress {
                let data = response.data.clone().unwrap_or(json!({}));
                eprintln!("[tabctl] progress: {}", data);
            }
            continue;
        }
        return Ok(response);
    }
    Err("No response received".to_string())
}

#[cfg(target_os = "linux")]
fn send_request_over_wsl_named_pipe(
    pipe_path: &str,
    action: &str,
    params: Value,
    show_progress: bool,
) -> Result<ResponseEnvelope, String> {
    let request = RequestEnvelope {
        id: Some(request_id()),
        action: action.to_string(),
        params,
        auth_token: None,
    };
    let request_json =
        serde_json::to_string(&request).map_err(|e| format!("Failed to encode request: {e}"))?;
    let escaped_request = request_json.replace('\'', "''");
    let pipe_name = wsl_named_pipe_name(pipe_path)?;
    let escaped_pipe_name = pipe_name.replace('\'', "''");
    let timeout_ms = response_timeout_ms();
    let script = format!(
        "$ErrorActionPreference='Stop';\
         [Console]::OutputEncoding=[System.Text.UTF8Encoding]::new($false);\
         $pipeName='{escaped_pipe_name}';\
         $request='{escaped_request}';\
         $pipe=[System.IO.Pipes.NamedPipeClientStream]::new('.',$pipeName,[System.IO.Pipes.PipeDirection]::InOut);\
         $pipe.Connect({timeout_ms});\
         $writer=[System.IO.StreamWriter]::new($pipe,[System.Text.UTF8Encoding]::new($false),4096,$true);\
         $writer.AutoFlush=$true;\
         $reader=[System.IO.StreamReader]::new($pipe,[System.Text.UTF8Encoding]::new($false),$false,4096,$true);\
         $writer.WriteLine($request);\
         while(($line=$reader.ReadLine()) -ne $null) {{ [Console]::WriteLine($line) }};",
    );

    let mut child = std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &script,
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| format!("Failed to start WSL named-pipe bridge via powershell.exe: {e}"))?;

    let start = std::time::Instant::now();
    loop {
        if child
            .try_wait()
            .map_err(|e| format!("Failed to poll WSL named-pipe bridge: {e}"))?
            .is_some()
        {
            break;
        }
        if start.elapsed() > Duration::from_millis(timeout_ms) {
            let _ = child.kill();
            let output = child
                .wait_with_output()
                .map_err(|e| format!("Failed to capture timed-out WSL pipe bridge output: {e}"))?;
            return Err(format!(
                "Request timed out after {timeout_ms}ms.\nstdout: {}\nstderr: {}",
                String::from_utf8_lossy(&output.stdout).trim(),
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        thread::sleep(Duration::from_millis(25));
    }

    let output = child
        .wait_with_output()
        .map_err(|e| format!("Failed to capture WSL named-pipe bridge output: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "WSL named-pipe bridge failed with status {}.\nstdout: {}\nstderr: {}",
            output.status,
            String::from_utf8_lossy(&output.stdout).trim(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    parse_response_lines(&String::from_utf8_lossy(&output.stdout), show_progress)
}

pub(super) fn send_request_over_stream<S>(
    mut stream: S,
    action: &str,
    params: Value,
    show_progress: bool,
    auth_token: Option<String>,
) -> Result<ResponseEnvelope, String>
where
    S: std::io::Read + Write + Send + 'static,
{
    let request = RequestEnvelope {
        id: Some(request_id()),
        action: action.to_string(),
        params,
        auth_token,
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
