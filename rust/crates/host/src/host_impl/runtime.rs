use std::fs;
use std::io::{self, IsTerminal};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::{Arc, Mutex};
use std::thread;
use tabctl_shared::{
    normalize_path_for_current_platform, path_to_platform_string, ProfileRegistry, SocketEndpoint,
    TabctlConfig,
};

use super::dispatch::{
    handle_client, start_native_reader, start_request_timeout_reaper, ClientWriter, Clients,
    NativeWriter,
};
use super::protocol::{log_line, next_counter};
use super::state::HostState;

// Keep the existing macOS paths so installed profiles and wrappers remain usable.
fn default_config_base() -> PathBuf {
    std::env::var("HOME")
        .map(|home| PathBuf::from(home).join(".config"))
        .unwrap_or_else(|_| PathBuf::from("."))
}

fn default_state_base() -> PathBuf {
    std::env::var("HOME")
        .map(|home| PathBuf::from(home).join(".local").join("state"))
        .unwrap_or_else(|_| PathBuf::from("."))
}

fn resolve_base_data_dir() -> PathBuf {
    if let Ok(path) = std::env::var("TABCTL_DATA_DIR") {
        if !path.trim().is_empty() {
            return PathBuf::from(normalize_path_for_current_platform(&path));
        }
    }
    if let Ok(path) = std::env::var("TABCTL_STATE_DIR") {
        if !path.trim().is_empty() {
            return PathBuf::from(normalize_path_for_current_platform(&path));
        }
    }
    if let Ok(state_home) = std::env::var("XDG_STATE_HOME") {
        return PathBuf::from(state_home).join("tabctl");
    }
    default_state_base().join("tabctl")
}

fn resolve_active_profile_name() -> Option<String> {
    std::env::var("TABCTL_PROFILE")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn resolve_profile_data_dir(config_dir: &Path, profile_name: &str) -> Option<PathBuf> {
    let profiles_path = config_dir.join("profiles.json");
    let content = fs::read_to_string(&profiles_path).ok()?;
    let registry = serde_json::from_str::<ProfileRegistry>(&content).ok()?;
    registry
        .profiles
        .get(profile_name)
        .map(|entry| PathBuf::from(normalize_path_for_current_platform(&entry.data_dir)))
}

pub(super) fn resolve_config() -> TabctlConfig {
    let config_dir = std::env::var("TABCTL_CONFIG_DIR")
        .map(|path| PathBuf::from(normalize_path_for_current_platform(&path)))
        .unwrap_or_else(|_| {
            std::env::var("XDG_CONFIG_HOME")
                .map(|path| PathBuf::from(normalize_path_for_current_platform(&path)))
                .unwrap_or_else(|_| default_config_base())
                .join("tabctl")
        });

    let base_data_dir = resolve_base_data_dir();
    let active_profile_name = resolve_active_profile_name();
    let data_dir = active_profile_name
        .as_deref()
        .and_then(|profile_name| resolve_profile_data_dir(&config_dir, profile_name))
        .unwrap_or_else(|| base_data_dir.clone());
    let socket_path = std::env::var("TABCTL_SOCKET")
        .unwrap_or_else(|_| path_to_platform_string(&data_dir.join("tabctl.sock")));

    TabctlConfig {
        config_dir: path_to_platform_string(&config_dir),
        data_dir: path_to_platform_string(&data_dir),
        base_data_dir: path_to_platform_string(&base_data_dir),
        socket_path,
        undo_log: path_to_platform_string(&data_dir.join("undo.jsonl")),
        wrapper_dir: path_to_platform_string(&data_dir),
        policy_path: path_to_platform_string(&config_dir.join("policy.json")),
        active_profile_name,
    }
}

fn run_host() -> io::Result<()> {
    if std::env::var("TABCTL_HOST_TCP").is_ok_and(|value| value == "1") {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "TABCTL_HOST_TCP is no longer supported; tabctl uses Unix-domain sockets only",
        ));
    }
    let config = resolve_config();
    let SocketEndpoint::Unix { path } = SocketEndpoint::parse(&config.socket_path)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err))?;
    let socket_path = PathBuf::from(path);
    fs::create_dir_all(&config.data_dir)?;

    if socket_path.exists() {
        let _ = fs::remove_file(&socket_path);
    }

    let listener = UnixListener::bind(&socket_path)?;
    fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600))?;

    let native_channel_available = !io::stdin().is_terminal() && !io::stdout().is_terminal();
    let state = Arc::new(Mutex::new(
        HostState::new_with_native_channel_and_page_cache(
            PathBuf::from(&config.undo_log),
            PathBuf::from(&config.base_data_dir).join("focus.db"),
            PathBuf::from(&config.data_dir).join("page-cache"),
            config.active_profile_name.clone(),
            native_channel_available,
        ),
    ));
    let clients: Clients = Arc::new(Mutex::new(std::collections::HashMap::new()));
    let native_out: NativeWriter = Arc::new(Mutex::new(Box::new(io::stdout())));
    start_native_reader(state.clone(), clients.clone(), native_out.clone());
    start_request_timeout_reaper(state.clone(), clients.clone(), native_out.clone());

    log_line(&format!("listening on {}", socket_path.display()));
    for incoming in listener.incoming() {
        match incoming {
            Ok(stream) => {
                let client_id = next_counter();
                let writer_stream = match stream.try_clone() {
                    Ok(clone) => clone,
                    Err(err) => {
                        log_line(&format!("socket clone error: {err}"));
                        continue;
                    }
                };
                let writer: ClientWriter = Arc::new(Mutex::new(Box::new(writer_stream)));
                if let Ok(mut map) = clients.lock() {
                    map.insert(client_id, writer);
                }
                let state_clone = state.clone();
                let clients_clone = clients.clone();
                let native_out_clone = native_out.clone();
                thread::spawn(move || {
                    handle_client(
                        client_id,
                        Box::new(stream),
                        state_clone,
                        clients_clone,
                        native_out_clone,
                    )
                });
            }
            Err(err) => log_line(&format!("socket accept error: {err}")),
        }
    }
    Ok(())
}

pub(super) fn run() {
    if let Err(err) = run_host() {
        log_line(&format!("fatal: {err}"));
        process::exit(1);
    }
}
