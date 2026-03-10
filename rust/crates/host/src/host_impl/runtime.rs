use sha2::{Digest, Sha256};
use std::fs;
#[cfg(windows)]
use std::fs::File;
use std::io;
use std::net::TcpListener;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(unix)]
use std::os::unix::net::UnixListener;
#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;
#[cfg(windows)]
use std::os::windows::io::{FromRawHandle, RawHandle};
use std::path::{Path, PathBuf};
use std::process;
use std::sync::{Arc, Mutex};
use std::thread;
#[cfg(windows)]
use tabctl_shared::windows_pipe_path;
use tabctl_shared::{
    normalize_path_for_current_platform, path_to_platform_string, ProfileRegistry, SocketEndpoint,
    TabctlConfig,
};
#[cfg(windows)]
use windows_sys::Win32::Foundation::{CloseHandle, ERROR_PIPE_CONNECTED, INVALID_HANDLE_VALUE};
#[cfg(windows)]
use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX};
#[cfg(windows)]
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE,
    PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};

use super::dispatch::{handle_client, start_native_reader, ClientWriter, Clients};
use super::protocol::{log_line, next_counter};
use super::state::HostState;

pub(super) const TCP_PORT_FILENAME: &str = "tcp-port";
pub(super) const AUTH_TOKEN_FILENAME: &str = "auth-token";
pub(super) const AUTH_TOKEN_LENGTH: usize = 32; // 32 hex chars = 128 bits
const TCP_PORT_BASE: u16 = 38_000;
const TCP_PORT_SPAN: u16 = 1_000;
const TCP_PORT_ATTEMPTS: u16 = 128;

fn default_config_base() -> PathBuf {
    #[cfg(windows)]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
            return PathBuf::from(appdata);
        }
        if let Ok(home) = std::env::var("USERPROFILE") {
            return PathBuf::from(home).join("AppData").join("Roaming");
        }
    }
    #[cfg(not(windows))]
    {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(".config");
        }
    }
    PathBuf::from(".")
}

fn default_state_base() -> PathBuf {
    #[cfg(windows)]
    {
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            return PathBuf::from(local);
        }
        if let Ok(home) = std::env::var("USERPROFILE") {
            return PathBuf::from(home).join("AppData").join("Local");
        }
    }
    #[cfg(not(windows))]
    {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(".local").join("state");
        }
    }
    PathBuf::from(".")
}

fn resolve_socket_path(data_dir: &Path) -> String {
    #[cfg(windows)]
    {
        windows_pipe_path(&path_to_platform_string(data_dir))
    }
    #[cfg(not(windows))]
    {
        path_to_platform_string(&data_dir.join("tabctl.sock"))
    }
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

    let socket_path =
        std::env::var("TABCTL_SOCKET").unwrap_or_else(|_| resolve_socket_path(&data_dir));

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

#[cfg(unix)]
fn run_unix() -> io::Result<()> {
    let config = resolve_config();
    let endpoint = SocketEndpoint::parse(&config.socket_path)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err))?;
    let socket_path = match endpoint {
        SocketEndpoint::Unix { path } => PathBuf::from(path),
        SocketEndpoint::Pipe { .. } => {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Named pipe endpoint is unsupported by the Unix host runtime",
            ));
        }
        SocketEndpoint::Tcp { .. } => {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "TCP endpoint is unsupported by the Unix host runtime",
            ));
        }
    };
    let socket_dir = PathBuf::from(&config.data_dir);
    fs::create_dir_all(&socket_dir)?;

    if socket_path.exists() {
        let _ = fs::remove_file(&socket_path);
    }

    let listener = UnixListener::bind(&socket_path)?;
    let _ = fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600));

    let state = Arc::new(Mutex::new(HostState::new(
        PathBuf::from(&config.undo_log),
        PathBuf::from(&config.base_data_dir).join("focus.db"),
        config.active_profile_name.clone(),
    )));
    let clients: Clients = Arc::new(Mutex::new(std::collections::HashMap::new()));
    let native_out = Arc::new(Mutex::new(io::stdout()));

    start_native_reader(state.clone(), clients.clone(), native_out.clone());

    // Optional TCP listener (opt-in via TABCTL_HOST_TCP=1)
    let tcp_enabled = std::env::var("TABCTL_HOST_TCP")
        .map(|v| v == "1")
        .unwrap_or(false);

    if tcp_enabled {
        let data_dir = PathBuf::from(&config.data_dir);
        let (tcp_listener, _tcp_port, auth_token) = setup_tcp_listener(&data_dir)?;
        spawn_tcp_accept_loop(
            tcp_listener,
            state.clone(),
            clients.clone(),
            native_out.clone(),
            auth_token,
        );
    }

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
                        None,
                    )
                });
            }
            Err(err) => log_line(&format!("socket accept error: {err}")),
        }
    }

    Ok(())
}

#[cfg(windows)]
fn to_wide(value: &str) -> Vec<u16> {
    std::ffi::OsStr::new(value)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(windows)]
fn connect_named_pipe_instance(path: &str) -> io::Result<File> {
    let mut open_mode = PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE;
    loop {
        let wide = to_wide(path);
        let handle = unsafe {
            CreateNamedPipeW(
                wide.as_ptr(),
                open_mode,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                PIPE_UNLIMITED_INSTANCES,
                64 * 1024,
                64 * 1024,
                0,
                std::ptr::null(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        let connected = unsafe { ConnectNamedPipe(handle, std::ptr::null_mut()) };
        if connected == 0 {
            let err = io::Error::last_os_error();
            if err.raw_os_error() != Some(ERROR_PIPE_CONNECTED as i32) {
                unsafe {
                    CloseHandle(handle);
                }
                if open_mode & FILE_FLAG_FIRST_PIPE_INSTANCE != 0 {
                    open_mode = PIPE_ACCESS_DUPLEX;
                    continue;
                }
                return Err(err);
            }
        }
        return Ok(unsafe { File::from_raw_handle(handle as RawHandle) });
    }
}

fn deterministic_tcp_start_port(data_dir: &Path) -> u16 {
    let mut hasher = Sha256::new();
    hasher.update(path_to_platform_string(data_dir).as_bytes());
    let digest = hasher.finalize();
    let seed = u16::from_be_bytes([digest[6], digest[7]]);
    TCP_PORT_BASE + (seed % TCP_PORT_SPAN)
}

fn bind_tcp_listener(data_dir: &Path) -> io::Result<(TcpListener, u16)> {
    if let Ok(port) = std::env::var("TABCTL_TCP_PORT") {
        let parsed = port
            .trim()
            .parse::<u16>()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Invalid TABCTL_TCP_PORT"))?;
        let listener = TcpListener::bind(("127.0.0.1", parsed))?;
        return Ok((listener, parsed));
    }

    let start = deterministic_tcp_start_port(data_dir);
    for offset in 0..TCP_PORT_ATTEMPTS {
        let candidate = TCP_PORT_BASE + ((start - TCP_PORT_BASE + offset) % TCP_PORT_SPAN);
        match TcpListener::bind(("127.0.0.1", candidate)) {
            Ok(listener) => return Ok((listener, candidate)),
            Err(err) if err.kind() == io::ErrorKind::AddrInUse => continue,
            Err(err) => return Err(err),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AddrInUse,
        "Failed to bind localhost TCP listener",
    ))
}

fn write_tcp_port_file(data_dir: &Path, port: u16) -> io::Result<PathBuf> {
    let path = data_dir.join(TCP_PORT_FILENAME);
    fs::write(&path, format!("{port}\n"))?;
    Ok(path)
}

pub(super) fn generate_and_write_auth_token(data_dir: &Path) -> io::Result<String> {
    let mut bytes = [0u8; 16]; // 16 bytes = 128 bits → 32 hex chars
    getrandom::getrandom(&mut bytes).map_err(|e| io::Error::other(e.to_string()))?;
    let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    debug_assert_eq!(token.len(), AUTH_TOKEN_LENGTH);
    let path = data_dir.join(AUTH_TOKEN_FILENAME);
    fs::write(&path, &token)?;
    Ok(token)
}

fn setup_tcp_listener(data_dir: &Path) -> io::Result<(TcpListener, u16, Arc<String>)> {
    let (listener, port) = bind_tcp_listener(data_dir)?;
    let port_file = write_tcp_port_file(data_dir, port)?;
    let auth_token = Arc::new(generate_and_write_auth_token(data_dir)?);
    log_line(&format!(
        "generated auth token in {}",
        data_dir.join(AUTH_TOKEN_FILENAME).display()
    ));
    log_line(&format!("listening on tcp://127.0.0.1:{port}"));
    log_line(&format!("published tcp port file {}", port_file.display()));
    Ok((listener, port, auth_token))
}

fn spawn_tcp_accept_loop(
    listener: TcpListener,
    state: Arc<Mutex<HostState>>,
    clients: Clients,
    native_out: Arc<Mutex<io::Stdout>>,
    auth_token: Arc<String>,
) {
    thread::spawn(move || {
        for incoming in listener.incoming() {
            match incoming {
                Ok(stream) => {
                    let client_id = next_counter();
                    let writer_stream = match stream.try_clone() {
                        Ok(clone) => clone,
                        Err(err) => {
                            log_line(&format!("tcp clone error: {err}"));
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
                    let token_clone = Some(auth_token.clone());
                    thread::spawn(move || {
                        handle_client(
                            client_id,
                            Box::new(stream),
                            state_clone,
                            clients_clone,
                            native_out_clone,
                            token_clone,
                        )
                    });
                }
                Err(err) => log_line(&format!("tcp accept error: {err}")),
            }
        }
    });
}

#[cfg(windows)]
fn run_windows() -> io::Result<()> {
    let config = resolve_config();
    let endpoint = SocketEndpoint::parse(&config.socket_path)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err))?;
    let pipe_path = match endpoint {
        SocketEndpoint::Pipe { path } => path,
        SocketEndpoint::Unix { .. } => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Windows host requires a named pipe endpoint",
            ));
        }
        SocketEndpoint::Tcp { .. } => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Windows host socket endpoint must be a named pipe",
            ));
        }
    };

    let data_dir = PathBuf::from(&config.data_dir);
    fs::create_dir_all(&data_dir)?;
    let (tcp_listener, _tcp_port, auth_token) = setup_tcp_listener(&data_dir)?;

    let state = Arc::new(Mutex::new(HostState::new(
        PathBuf::from(&config.undo_log),
        PathBuf::from(&config.base_data_dir).join("focus.db"),
        config.active_profile_name.clone(),
    )));
    let clients: Clients = Arc::new(Mutex::new(std::collections::HashMap::new()));
    let native_out = Arc::new(Mutex::new(io::stdout()));
    start_native_reader(state.clone(), clients.clone(), native_out.clone());

    spawn_tcp_accept_loop(
        tcp_listener,
        state.clone(),
        clients.clone(),
        native_out.clone(),
        auth_token,
    );

    log_line(&format!("listening on {pipe_path}"));

    loop {
        match connect_named_pipe_instance(&pipe_path) {
            Ok(pipe) => {
                let client_id = next_counter();
                let reader = match pipe.try_clone() {
                    Ok(clone) => clone,
                    Err(err) => {
                        log_line(&format!("pipe clone error: {err}"));
                        continue;
                    }
                };
                let writer: ClientWriter = Arc::new(Mutex::new(Box::new(pipe)));
                if let Ok(mut map) = clients.lock() {
                    map.insert(client_id, writer);
                }
                let state_clone = state.clone();
                let clients_clone = clients.clone();
                let native_out_clone = native_out.clone();
                thread::spawn(move || {
                    handle_client(
                        client_id,
                        Box::new(reader),
                        state_clone,
                        clients_clone,
                        native_out_clone,
                        None,
                    )
                });
            }
            Err(err) => log_line(&format!("named pipe accept error: {err}")),
        }
    }
}

#[cfg(not(any(unix, windows)))]
fn run_host() -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Rust host runtime is unsupported on this target",
    ))
}

#[cfg(unix)]
fn run_host() -> io::Result<()> {
    run_unix()
}

#[cfg(windows)]
fn run_host() -> io::Result<()> {
    run_windows()
}

pub(super) fn run() {
    if let Err(err) = run_host() {
        log_line(&format!("fatal: {err}"));
        process::exit(1);
    }
}

// Compile-time invariants for the TCP port range constants shared by host and CLI.
const _: () = assert!(
    TCP_PORT_BASE >= 1024,
    "port base must be an unprivileged port"
);
const _: () = assert!(TCP_PORT_SPAN > 0, "port span must be non-zero");
const _: () = assert!(
    (TCP_PORT_BASE as u32) + (TCP_PORT_SPAN as u32) <= 65535,
    "port range must fit in a u16"
);
