use super::*;

pub(super) fn resolve_socket_endpoint(profile: Option<&str>) -> Result<SocketEndpoint, String> {
    if let Ok(path) = std::env::var("TABCTL_SOCKET") {
        if !path.trim().is_empty() {
            let endpoint = SocketEndpoint::parse(&path)?;
            #[cfg(target_os = "linux")]
            if matches!(endpoint, SocketEndpoint::Pipe { .. }) && is_wsl_environment() {
                if let Some(tcp) = discover_wsl_tcp_endpoint(profile) {
                    return Ok(tcp);
                }
            }
            return Ok(endpoint);
        }
    }
    // Check TABCTL_TRANSPORT=tcp for explicit TCP transport selection
    if let Ok(transport) = std::env::var("TABCTL_TRANSPORT") {
        if transport.trim().eq_ignore_ascii_case("tcp") {
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
        if let Some(tcp) = discover_wsl_tcp_endpoint(profile) {
            return Ok(tcp);
        }
        return Ok(SocketEndpoint::Tcp {
            host: "127.0.0.1".to_string(),
            port: WSL_TCP_PORT_FALLBACK,
        });
    }
    let data_dir = resolve_data_dir(profile)?;
    #[cfg(windows)]
    {
        Ok(resolve_windows_pipe_endpoint(&data_dir))
    }
    #[cfg(not(windows))]
    {
        SocketEndpoint::parse(&format!("{data_dir}/tabctl.sock"))
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

#[cfg(target_os = "linux")]
pub(super) fn discover_wsl_tcp_endpoint(profile: Option<&str>) -> Option<SocketEndpoint> {
    if let Ok(value) = std::env::var("TABCTL_TCP_PORT") {
        if let Ok(port) = value.trim().parse::<u16>() {
            if port > 0 {
                return Some(SocketEndpoint::Tcp {
                    host: "127.0.0.1".to_string(),
                    port,
                });
            }
        }
    }
    let data_dir = resolve_data_dir(profile).ok()?;
    discover_wsl_tcp_port_from_data_dir(&data_dir).map(|port| SocketEndpoint::Tcp {
        host: "127.0.0.1".to_string(),
        port,
    })
}

#[cfg(target_os = "linux")]
pub(super) fn discover_wsl_tcp_port_from_data_dir(data_dir: &str) -> Option<u16> {
    for path in wsl_tcp_port_candidates(data_dir) {
        if let Some(port) = read_tcp_port_file(&path) {
            return Some(port);
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
    let path = PathBuf::from(format!("/mnt/c/Users/{username}/AppData/Local"));
    if path.is_dir() {
        Some(path)
    } else {
        None
    }
}

#[cfg(any(target_os = "linux", test))]
pub(super) fn wsl_file_candidates(data_dir: &str, filename: &str) -> Vec<PathBuf> {
    let mut candidates = vec![PathBuf::from(data_dir).join(filename)];
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

#[cfg(target_os = "linux")]
pub(super) fn wsl_tcp_port_candidates(data_dir: &str) -> Vec<PathBuf> {
    wsl_file_candidates(data_dir, WSL_TCP_PORT_FILENAME)
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
    let mut hasher = Sha256::new();
    hasher.update(data_dir.as_bytes());
    let digest = hasher.finalize();
    let hash = digest[..6]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    SocketEndpoint::Pipe {
        path: format!(r"\\.\pipe\tabctl-{hash}"),
    }
}

pub(super) fn resolve_data_dir(profile: Option<&str>) -> Result<String, String> {
    if let Ok(path) = std::env::var("TABCTL_DATA_DIR") {
        if !path.trim().is_empty() {
            return Ok(path);
        }
    }
    let config_dir = resolve_config_dir()?;
    if let Some(profile_name) = profile {
        let profiles_path = PathBuf::from(&config_dir).join("profiles.json");
        if let Ok(contents) = fs::read_to_string(profiles_path) {
            if let Ok(registry) = serde_json::from_str::<ProfileRegistry>(&contents) {
                if let Some(profile_entry) = registry.profiles.get(profile_name) {
                    return Ok(profile_entry.data_dir.clone());
                }
            }
        }
        return Err(format!(
            "Profile \"{profile_name}\" not found in profiles.json"
        ));
    }
    if let Ok(path) = std::env::var("TABCTL_STATE_DIR") {
        if !path.trim().is_empty() {
            return Ok(path);
        }
    }
    if let Ok(path) = std::env::var("XDG_STATE_HOME") {
        return Ok(format!("{path}/tabctl"));
    }
    let home = dirs::home_dir().ok_or_else(|| "Unable to resolve home directory".to_string())?;
    Ok(format!("{}/.local/state/tabctl", home.display()))
}

pub(super) fn resolve_config_dir() -> Result<String, String> {
    if let Ok(path) = std::env::var("TABCTL_CONFIG_DIR") {
        return Ok(path);
    }
    if let Ok(path) = std::env::var("XDG_CONFIG_HOME") {
        return Ok(format!("{path}/tabctl"));
    }
    let home = dirs::home_dir().ok_or_else(|| "Unable to resolve home directory".to_string())?;
    Ok(format!("{}/.config/tabctl", home.display()))
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
                let stream = fs::OpenOptions::new()
                    .read(true)
                    .write(true)
                    .open(path)
                    .map_err(|e| format!("Failed to connect to host: {e}"))?;
                send_request_over_stream(stream, action, params, show_progress, None)
            }
            #[cfg(not(windows))]
            {
                let _ = path;
                Err("Named pipe transport is unsupported on this target".to_string())
            }
        }
    }
}

pub(super) fn send_request_over_stream<S>(
    mut stream: S,
    action: &str,
    params: Value,
    show_progress: bool,
    auth_token: Option<String>,
) -> Result<ResponseEnvelope, String>
where
    S: std::io::Read + Write,
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

    let reader = BufReader::new(stream);
    for line in reader.lines() {
        let line = line.map_err(|e| format!("Failed to read response: {e}"))?;
        if line.trim().is_empty() {
            continue;
        }
        let response: ResponseEnvelope =
            serde_json::from_str(&line).map_err(|e| format!("Invalid response payload: {e}"))?;
        if response.progress.unwrap_or(false) {
            if show_progress {
                let data = response.data.unwrap_or(json!({}));
                eprintln!("[tabctl] progress: {}", data);
            }
            continue;
        }
        return Ok(response);
    }
    Err("No response received".to_string())
}
