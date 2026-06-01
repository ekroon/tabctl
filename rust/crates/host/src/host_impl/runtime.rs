#[cfg(not(windows))]
use sha2::{Digest, Sha256};
use std::fs;
#[cfg(windows)]
use std::fs::File;
use std::io;
use std::io::IsTerminal;
#[cfg(not(windows))]
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
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_ACCESS_DENIED, ERROR_PIPE_CONNECTED, INVALID_HANDLE_VALUE,
};
#[cfg(windows)]
use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX};
#[cfg(windows)]
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE,
    PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};

use super::dispatch::{
    handle_client, start_native_reader, start_request_timeout_reaper, ClientWriter, Clients,
    NativeWriter,
};
use super::protocol::{log_line, next_counter};
use super::state::HostState;

#[cfg(not(windows))]
pub(super) const TCP_PORT_FILENAME: &str = "tcp-port";
#[cfg(not(windows))]
pub(super) const AUTH_TOKEN_FILENAME: &str = "auth-token";
#[cfg(windows)]
pub(super) const PIPE_ENDPOINT_FILENAME: &str = "pipe-endpoint";
#[cfg(not(windows))]
pub(super) const AUTH_TOKEN_LENGTH: usize = 32; // 32 hex chars = 128 bits
#[cfg(not(windows))]
const TCP_PORT_BASE: u16 = 38_000;
#[cfg(not(windows))]
const TCP_PORT_SPAN: u16 = 1_000;
#[cfg(not(windows))]
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
                        true,
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
fn should_retry_without_first_pipe_instance(open_mode: u32, err: &io::Error) -> bool {
    open_mode & FILE_FLAG_FIRST_PIPE_INSTANCE != 0
        && err.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32)
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
            let err = io::Error::last_os_error();
            if should_retry_without_first_pipe_instance(open_mode, &err) {
                open_mode = PIPE_ACCESS_DUPLEX;
                continue;
            }
            return Err(err);
        }
        let connected = unsafe { ConnectNamedPipe(handle, std::ptr::null_mut()) };
        if connected == 0 {
            let err = io::Error::last_os_error();
            if err.raw_os_error() != Some(ERROR_PIPE_CONNECTED as i32) {
                unsafe {
                    CloseHandle(handle);
                }
                return Err(err);
            }
        }
        return Ok(unsafe { File::from_raw_handle(handle as RawHandle) });
    }
}

#[cfg(not(windows))]
fn deterministic_tcp_start_port(data_dir: &Path) -> u16 {
    let mut hasher = Sha256::new();
    hasher.update(path_to_platform_string(data_dir).as_bytes());
    let digest = hasher.finalize();
    let seed = u16::from_be_bytes([digest[6], digest[7]]);
    TCP_PORT_BASE + (seed % TCP_PORT_SPAN)
}

#[cfg(not(windows))]
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

#[cfg(not(windows))]
fn write_tcp_port_file(data_dir: &Path, port: u16) -> io::Result<PathBuf> {
    let path = data_dir.join(TCP_PORT_FILENAME);
    fs::write(&path, format!("{port}\n"))?;
    Ok(path)
}

#[cfg(windows)]
fn write_pipe_endpoint_file(data_dir: &Path, pipe_path: &str) -> io::Result<PathBuf> {
    let path = data_dir.join(PIPE_ENDPOINT_FILENAME);
    fs::write(&path, format!("{pipe_path}\n"))?;
    Ok(path)
}

#[cfg(not(windows))]
pub(super) fn generate_and_write_auth_token(data_dir: &Path) -> io::Result<String> {
    let mut bytes = [0u8; 16]; // 16 bytes = 128 bits → 32 hex chars
    getrandom::getrandom(&mut bytes).map_err(|e| io::Error::other(e.to_string()))?;
    let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    debug_assert_eq!(token.len(), AUTH_TOKEN_LENGTH);
    let path = data_dir.join(AUTH_TOKEN_FILENAME);
    fs::write(&path, &token)?;
    Ok(token)
}

#[cfg(not(windows))]
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

#[cfg(not(windows))]
fn spawn_tcp_accept_loop(
    listener: TcpListener,
    state: Arc<Mutex<HostState>>,
    clients: Clients,
    native_out: NativeWriter,
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
                            true,
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
    let pipe_file = write_pipe_endpoint_file(&data_dir, &pipe_path)?;

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

    log_line(&format!(
        "published pipe endpoint file {}",
        pipe_file.display()
    ));
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
                        true,
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
#[cfg(not(windows))]
const _: () = assert!(
    TCP_PORT_BASE >= 1024,
    "port base must be an unprivileged port"
);
#[cfg(not(windows))]
const _: () = assert!(TCP_PORT_SPAN > 0, "port span must be non-zero");
#[cfg(not(windows))]
const _: () = assert!(
    (TCP_PORT_BASE as u32) + (TCP_PORT_SPAN as u32) <= 65535,
    "port range must fit in a u16"
);

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use super::*;

    #[cfg(windows)]
    fn test_pipe_path(name: &str) -> String {
        format!(r"\\.\pipe\tabctl-{name}-{}", next_counter())
    }

    #[cfg(windows)]
    #[test]
    fn retries_without_first_pipe_instance_on_access_denied() {
        let err = io::Error::from_raw_os_error(ERROR_ACCESS_DENIED as i32);
        assert!(should_retry_without_first_pipe_instance(
            PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
            &err,
        ));
    }

    #[cfg(windows)]
    #[test]
    fn does_not_retry_without_first_pipe_instance_for_other_errors() {
        let err = io::Error::from_raw_os_error(ERROR_PIPE_CONNECTED as i32);
        assert!(!should_retry_without_first_pipe_instance(
            PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
            &err,
        ));
    }

    #[cfg(windows)]
    #[test]
    fn does_not_retry_when_first_pipe_instance_flag_is_not_set() {
        let err = io::Error::from_raw_os_error(ERROR_ACCESS_DENIED as i32);
        assert!(!should_retry_without_first_pipe_instance(
            PIPE_ACCESS_DUPLEX,
            &err
        ));
    }

    #[cfg(windows)]
    #[test]
    fn named_pipe_roundtrip_survives_server_reader_drop_after_request() {
        use std::fs::OpenOptions;
        use std::io::{BufRead, BufReader, Write};
        use std::thread;
        use std::time::Duration;

        let pipe_path = test_pipe_path("roundtrip");
        let server_path = pipe_path.clone();
        let server = thread::spawn(move || {
            let mut writer = connect_named_pipe_instance(&server_path).expect("create server pipe");
            let reader = writer.try_clone().expect("clone server pipe for reader");
            let mut buf = String::new();
            let mut reader = BufReader::new(reader);
            reader.read_line(&mut buf).expect("read client request");
            assert!(buf.contains("\"action\":\"snapshot\""));
            drop(reader);
            thread::sleep(Duration::from_millis(50));
            writeln!(writer, "{{\"ok\":true,\"action\":\"snapshot\",\"requestId\":\"req-1\",\"data\":{{\"windows\":[]}}}}")
                .expect("write server response");
            writer.flush().expect("flush server response");
        });

        let mut client = loop {
            match OpenOptions::new().read(true).write(true).open(&pipe_path) {
                Ok(client) => break client,
                Err(err) if err.raw_os_error() == Some(2) || err.raw_os_error() == Some(231) => {
                    thread::sleep(Duration::from_millis(10));
                    continue;
                }
                Err(err) => panic!("connect client to named pipe: {err}"),
            }
        };
        let mut client_reader = BufReader::new(client.try_clone().expect("clone client pipe"));
        writeln!(
            client,
            "{{\"id\":\"req-1\",\"action\":\"snapshot\",\"params\":{{}}}}"
        )
        .expect("write client request");
        client.flush().expect("flush client request");

        let mut response = String::new();
        client_reader
            .read_line(&mut response)
            .expect("read server response");
        assert!(response.contains("\"ok\":true"));
        assert!(response.contains("\"requestId\":\"req-1\""));

        server.join().expect("join server thread");
    }
}
