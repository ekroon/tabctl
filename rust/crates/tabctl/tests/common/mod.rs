//! Shared integration-test infrastructure.
//!
//! Provides helpers for running CLI commands and a `SharedBrowser` fixture that
//! boots Chrome + extension exactly once (via `OnceLock`) and exposes a
//! convenient API for browser-backed tests.

#![allow(dead_code)]

use serde_json::Value;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::OnceLock;
use std::thread::sleep;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tabctl_shared::{RequestEnvelope, ResponseEnvelope, SocketEndpoint};

#[cfg(not(windows))]
use std::net::TcpStream;
#[cfg(windows)]
use std::net::TcpStream;
#[cfg(unix)]
use std::os::unix::net::UnixStream;

// ── Utility helpers ─────────────────────────────────────────────────────────

pub fn repo_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .expect("failed to resolve repository root from CARGO_MANIFEST_DIR")
        .to_path_buf()
}

pub fn rust_tabctl_bin(root: &Path) -> PathBuf {
    let mut bin = root
        .join("rust")
        .join("target")
        .join("debug")
        .join("tabctl");
    if cfg!(windows) {
        bin.set_extension("exe");
    }
    bin
}

pub fn integration_bootstrap_script(root: &Path) -> PathBuf {
    std::env::var("TABCTL_INTEGRATION_BOOTSTRAP")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            root.join("scripts")
                .join("ci")
                .join("integration-bootstrap.js")
        })
}

pub fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_millis()
}

static SANDBOX_COUNTER: AtomicU32 = AtomicU32::new(0);

/// Create an isolated sandbox directory for tests. Caller is responsible for cleanup.
pub fn create_sandbox() -> PathBuf {
    let seq = SANDBOX_COUNTER.fetch_add(1, Ordering::Relaxed);
    let sandbox = std::env::temp_dir().join(format!(
        "tbi-local-{}-{}-{}",
        now_ms(),
        std::process::id(),
        seq
    ));
    fs::create_dir_all(&sandbox).expect("create test sandbox");
    sandbox
}

// ── JSON response helpers ───────────────────────────────────────────────────

pub fn assert_ok(action: &str, payload: &Value) {
    if payload.get("ok").is_some() {
        assert_eq!(
            payload.get("ok").and_then(Value::as_bool),
            Some(true),
            "tabctl {action} returned non-ok payload: {payload}"
        );
    } else {
        assert!(
            payload.get("error").is_none(),
            "tabctl {action} returned error payload: {payload}"
        );
    }
}

pub fn response_data(payload: &Value) -> &Value {
    payload.get("data").unwrap_or(payload)
}

pub fn gql_string(value: &str) -> String {
    serde_json::to_string(value).expect("serialize GraphQL string literal")
}

// ── RAII guards ─────────────────────────────────────────────────────────────

pub struct ChildGuard {
    child: Child,
}

impl ChildGuard {
    pub fn new(child: Child) -> Self {
        Self { child }
    }

    pub fn child_mut(&mut self) -> &mut Child {
        &mut self.child
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Ok(None) = self.child.try_wait() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

pub struct TempDirGuard {
    path: PathBuf,
}

impl TempDirGuard {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

// ── CLI runners ─────────────────────────────────────────────────────────────

pub fn run_tabctl_json_with_timeout(
    tabctl_bin: &Path,
    root: &Path,
    profile: &str,
    config_home: &Path,
    state_home: &Path,
    args: &[&str],
    timeout: Duration,
) -> Result<Value, String> {
    let mut command = Command::new(tabctl_bin);
    command
        .arg("--json")
        .arg("--no-pretty")
        .arg("--profile")
        .arg(profile)
        .args(args)
        .current_dir(root)
        .env("XDG_CONFIG_HOME", config_home)
        .env("XDG_STATE_HOME", state_home)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command.env_remove("TABCTL_TRANSPORT");
    command.env_remove("TABCTL_TCP_PORT");
    let mut child = command
        .spawn()
        .map_err(|e| format!("failed to execute tabctl {:?}: {e}", args))?;
    let start = Instant::now();
    loop {
        if child
            .try_wait()
            .map_err(|e| format!("failed to poll tabctl {:?}: {e}", args))?
            .is_some()
        {
            break;
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let output = child
                .wait_with_output()
                .map_err(|e| format!("failed to capture timed-out tabctl {:?}: {e}", args))?;
            return Err(format!(
                "tabctl {:?} timed out after {}s.\nstdout: {}\nstderr: {}",
                args,
                timeout.as_secs(),
                String::from_utf8_lossy(&output.stdout).trim(),
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        sleep(Duration::from_millis(100));
    }
    let output = child
        .wait_with_output()
        .map_err(|e| format!("failed to capture tabctl {:?} output: {e}", args))?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !output.status.success() {
        return Err(format!(
            "tabctl {:?} failed with status {}.\nstdout: {}\nstderr: {}",
            args, output.status, stdout, stderr
        ));
    }
    serde_json::from_str::<Value>(&stdout).map_err(|e| {
        format!(
            "tabctl {:?} returned non-JSON output ({e}). raw stdout: {}",
            args, stdout
        )
    })
}

pub fn run_tabctl_json(
    tabctl_bin: &Path,
    root: &Path,
    profile: &str,
    config_home: &Path,
    state_home: &Path,
    args: &[&str],
) -> Result<Value, String> {
    run_tabctl_json_with_timeout(
        tabctl_bin,
        root,
        profile,
        config_home,
        state_home,
        args,
        Duration::from_secs(30),
    )
}

/// Run a tabctl CLI command without --json. Returns (stdout, stderr).
pub fn run_tabctl_raw(tabctl_bin: &Path, args: &[&str]) -> Result<(String, String), String> {
    let output = Command::new(tabctl_bin)
        .args(args)
        .output()
        .map_err(|e| format!("failed to execute tabctl {:?}: {e}", args))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if !output.status.success() {
        return Err(format!(
            "tabctl {:?} failed with status {}.\nstdout: {}\nstderr: {}",
            args, output.status, stdout, stderr
        ));
    }
    Ok((stdout, stderr))
}

// ── Shared browser fixture ──────────────────────────────────────────────────

/// PID of the bootstrap child, stored for atexit cleanup.
static BOOTSTRAP_PID: AtomicU32 = AtomicU32::new(0);

#[cfg(unix)]
mod cleanup {
    extern "C" {
        pub fn atexit(func: extern "C" fn()) -> std::ffi::c_int;
        pub fn kill(pid: std::ffi::c_int, sig: std::ffi::c_int) -> std::ffi::c_int;
    }
    pub const SIGTERM: std::ffi::c_int = 15;
}

#[cfg(windows)]
mod cleanup {
    extern "C" {
        pub fn atexit(func: extern "C" fn()) -> std::ffi::c_int;
    }

    extern "system" {
        fn OpenProcess(
            desired_access: u32,
            inherit_handle: i32,
            process_id: u32,
        ) -> *mut std::ffi::c_void;
        fn TerminateProcess(handle: *mut std::ffi::c_void, exit_code: u32) -> i32;
        fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
    }

    /// Terminate the bootstrap process on Windows via `TerminateProcess`.
    pub fn terminate_pid(pid: u32) {
        const PROCESS_TERMINATE: u32 = 0x0001;
        unsafe {
            let handle = OpenProcess(PROCESS_TERMINATE, 0, pid);
            if !handle.is_null() {
                TerminateProcess(handle, 1);
                CloseHandle(handle);
            }
        }
    }
}

#[cfg(unix)]
extern "C" fn cleanup_bootstrap() {
    let pid = BOOTSTRAP_PID.load(Ordering::SeqCst);
    if pid != 0 && pid <= i32::MAX as u32 {
        unsafe {
            cleanup::kill(pid as std::ffi::c_int, cleanup::SIGTERM);
        }
    }
}

#[cfg(windows)]
extern "C" fn cleanup_bootstrap() {
    let pid = BOOTSTRAP_PID.load(Ordering::SeqCst);
    if pid != 0 {
        cleanup::terminate_pid(pid);
    }
}

/// Shared browser fixture: tabctl binary, profile, and sandbox paths.
///
/// Created once via `OnceLock` and reused across all browser-backed tests.
/// Tests should use `shared_browser()` to obtain a reference.
pub struct SharedBrowser {
    pub tabctl_bin: PathBuf,
    pub root: PathBuf,
    pub profile_name: String,
    pub config_home: PathBuf,
    pub state_home: PathBuf,
}

impl SharedBrowser {
    /// Run a tabctl CLI command with `--json` and return parsed JSON.
    pub fn run(&self, args: &[&str]) -> Value {
        run_tabctl_json(
            &self.tabctl_bin,
            &self.root,
            &self.profile_name,
            &self.config_home,
            &self.state_home,
            args,
        )
        .unwrap_or_else(|e| panic!("tabctl {:?} failed: {e}", args))
    }

    /// Run a tabctl CLI command, returning `Result` for fallible assertions.
    pub fn run_result(&self, args: &[&str]) -> Result<Value, String> {
        run_tabctl_json(
            &self.tabctl_bin,
            &self.root,
            &self.profile_name,
            &self.config_home,
            &self.state_home,
            args,
        )
    }

    /// Run a tabctl CLI command with a custom timeout.
    pub fn run_timeout(&self, args: &[&str], timeout: Duration) -> Result<Value, String> {
        run_tabctl_json_with_timeout(
            &self.tabctl_bin,
            &self.root,
            &self.profile_name,
            &self.config_home,
            &self.state_home,
            args,
            timeout,
        )
    }

    pub fn run_query(&self, query: &str) -> Value {
        self.run_query_result(query)
            .unwrap_or_else(|e| panic!("tabctl query failed: {e}\nquery: {query}"))
    }

    pub fn run_query_result(&self, query: &str) -> Result<Value, String> {
        let mut last_error = None;
        for attempt in 0..6 {
            match run_tabctl_json(
                &self.tabctl_bin,
                &self.root,
                &self.profile_name,
                &self.config_home,
                &self.state_home,
                &["query", query],
            ) {
                Ok(value) => return Ok(value),
                Err(err) if attempt < 5 && is_transient_host_error(&err) => {
                    last_error = Some(err);
                    sleep(Duration::from_millis(500));
                }
                Err(err) => return Err(err),
            }
        }
        Err(last_error.unwrap_or_else(|| "tabctl query failed".to_string()))
    }

    pub fn wait_for_host_ready(&self, timeout: Duration) {
        let start = Instant::now();
        let mut last_error = String::new();

        while start.elapsed() < timeout {
            match run_tabctl_json_with_timeout(
                &self.tabctl_bin,
                &self.root,
                &self.profile_name,
                &self.config_home,
                &self.state_home,
                &["ping"],
                Duration::from_secs(10),
            ) {
                Ok(ping) => {
                    if ping.get("ok").and_then(Value::as_bool) == Some(true)
                        || (ping.get("ok").is_none() && ping.get("error").is_none())
                    {
                        return;
                    }
                    last_error = format!("non-ok ping payload: {ping}");
                }
                Err(err) => {
                    last_error = err;
                }
            }
            sleep(Duration::from_millis(500));
        }

        panic!(
            "host did not become ready within {}s; last error: {last_error}",
            timeout.as_secs()
        );
    }

    /// Create an isolated test window with the given URLs and optional group.
    /// Returns `(window_id, created_tab_ids)`.
    pub fn create_test_window(&self, urls: &[&str], group: Option<&str>) -> (i64, Vec<i64>) {
        let urls = urls
            .iter()
            .map(|url| gql_string(url))
            .collect::<Vec<_>>()
            .join(", ");
        let open = self.run_query(&format!(
            "mutation {{ openTabs(urls: [{urls}], newWindow: true) {{ windowId tabs {{ tabId }} }} }}"
        ));
        assert_ok("create_test_window", &open);
        let data = response_data(&open);
        let window_id = data
            .pointer("/openTabs/windowId")
            .and_then(Value::as_i64)
            .expect("create_test_window: missing windowId");
        let tab_ids: Vec<i64> = data
            .pointer("/openTabs/tabs")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|tab| tab.get("tabId").and_then(Value::as_i64))
                    .collect()
            })
            .unwrap_or_default();
        if let Some(group_title) = group {
            let ids = tab_ids
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            let assign = self.run_query(&format!(
                "mutation {{ assignToGroup(tabIds: [{ids}], groupTitle: {}) {{ groupId title }} }}",
                gql_string(group_title)
            ));
            assert_ok("create_test_window assignToGroup", &assign);
        }
        (window_id, tab_ids)
    }

    /// Close a test window (best-effort cleanup, errors ignored).
    pub fn close_test_window(&self, window_id: i64) {
        let query =
            format!("query {{ tabs(windowId: {window_id}, limit: 200) {{ items {{ tabId }} }} }}");
        let Ok(payload) = self.run_query_result(&query) else {
            return;
        };
        let tab_ids: Vec<i64> = response_data(&payload)
            .pointer("/tabs/items")
            .and_then(Value::as_array)
            .map(|tabs| {
                tabs.iter()
                    .filter_map(|tab| tab.get("tabId").and_then(Value::as_i64))
                    .collect()
            })
            .unwrap_or_default();
        if tab_ids.is_empty() {
            return;
        }
        let ids = tab_ids
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        let _ = self.run_query_result(&format!(
            "mutation {{ closeTabs(tabIds: [{ids}], confirm: true) {{ txid closedTabs }} }}"
        ));
    }

    pub fn send_host_request(&self, action: &str, params: Value) -> Value {
        let profile = self.run(&["profile-show"]);
        let endpoint = response_data(&profile)
            .get("socket")
            .and_then(Value::as_str)
            .expect("profile-show should return socket uri");
        let endpoint = SocketEndpoint::parse(endpoint).expect("parse socket endpoint");
        let data_dir = response_data(&profile)
            .get("dataDir")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);
        let auth_token = match endpoint {
            SocketEndpoint::Tcp { .. } => data_dir.and_then(|dir| {
                fs::read_to_string(PathBuf::from(dir).join("auth-token"))
                    .ok()
                    .map(|token| token.trim().to_string())
                    .filter(|token| !token.is_empty())
            }),
            _ => None,
        };

        let request = RequestEnvelope {
            id: Some(format!("itest-{}", now_ms())),
            action: action.to_string(),
            params,
            auth_token,
        };

        let response = match endpoint {
            SocketEndpoint::Unix { path } => {
                #[cfg(unix)]
                {
                    let stream = UnixStream::connect(path).expect("connect unix socket");
                    send_host_request_over_stream(stream, &request)
                }
                #[cfg(not(unix))]
                {
                    let _ = path;
                    panic!("unix sockets are unsupported on this target")
                }
            }
            SocketEndpoint::Tcp { host, port } => {
                let stream = TcpStream::connect((host.as_str(), port)).expect("connect tcp socket");
                send_host_request_over_stream(stream, &request)
            }
            SocketEndpoint::Pipe { path } => {
                #[cfg(windows)]
                {
                    let stream = std::fs::OpenOptions::new()
                        .read(true)
                        .write(true)
                        .open(path)
                        .expect("connect named pipe");
                    send_host_request_over_stream(stream, &request)
                }
                #[cfg(not(windows))]
                {
                    let _ = path;
                    panic!("named pipes are unsupported on this target")
                }
            }
        };

        if response.ok {
            response.data.unwrap_or_else(|| serde_json::json!({}))
        } else {
            serde_json::json!({
                "ok": false,
                "error": {
                    "message": response.error.map(|err| err.message).unwrap_or_else(|| "request failed".to_string())
                }
            })
        }
    }
}

fn is_transient_host_error(err: &str) -> bool {
    err.contains("Failed to connect to host")
        || err.contains("Connection refused")
        || err.contains("ECONNREFUSED")
}

fn send_host_request_over_stream<S>(mut stream: S, request: &RequestEnvelope) -> ResponseEnvelope
where
    S: std::io::Read + Write,
{
    serde_json::to_writer(&mut stream, request).expect("encode host request");
    stream.write_all(b"\n").expect("write request terminator");
    stream.flush().expect("flush host request");

    let reader = BufReader::new(stream);
    for line in reader.lines() {
        let line = line.expect("read host response");
        if line.trim().is_empty() {
            continue;
        }
        let response: ResponseEnvelope = serde_json::from_str(&line).expect("decode host response");
        if response.progress.unwrap_or(false) {
            continue;
        }
        return response;
    }
    panic!("no host response received");
}

static BROWSER: OnceLock<SharedBrowser> = OnceLock::new();

fn init_browser() -> SharedBrowser {
    let root = repo_root();
    let script = integration_bootstrap_script(&root);
    assert!(
        script.exists(),
        "integration bootstrap not found at {}",
        script.display()
    );

    let tabctl_bin = rust_tabctl_bin(&root);
    assert!(
        tabctl_bin.exists(),
        "Rust tabctl binary not found at {} (run `npm run build` first)",
        tabctl_bin.display()
    );
    let extension_dir = root.join("dist").join("extension");
    assert!(
        extension_dir.join("manifest.json").exists(),
        "Built extension not found at {} (run `npm run build` first)",
        extension_dir.display()
    );

    let sandbox = if cfg!(windows) {
        std::env::temp_dir().join(format!("tbi-shared-{}", now_ms()))
    } else {
        PathBuf::from(format!("/tmp/tbi-shared-{}", now_ms()))
    };
    fs::create_dir_all(&sandbox).expect("create test sandbox");
    // NOTE: TempDirGuard is intentionally NOT used — static values never Drop.
    // The atexit handler cleans up the bootstrap process; the sandbox dir is
    // ephemeral and cleaned by CI or OS.
    let config_home = sandbox.join("c");
    let state_home = sandbox.join("s");
    fs::create_dir_all(&config_home).expect("create XDG config dir");
    fs::create_dir_all(&state_home).expect("create XDG state dir");

    let profile_name = "itest-chrome";

    let setup = run_tabctl_json(
        &tabctl_bin,
        &root,
        profile_name,
        &config_home,
        &state_home,
        &[
            "setup",
            "--browser",
            "chrome",
            "--name",
            profile_name,
            "--extension-dir",
            extension_dir.to_str().expect("extension path to utf8"),
        ],
    )
    .expect("setup command should succeed");
    assert_ok("setup", &setup);

    let setup_data = response_data(&setup);
    let active_extension_dir = setup_data
        .pointer("/extensionSync/activePath")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .unwrap_or(extension_dir);
    let host_wrapper = setup_data
        .pointer("/wrapperPath")
        .and_then(Value::as_str)
        .expect("setup payload missing data.wrapperPath");
    assert!(
        active_extension_dir.join("manifest.json").exists(),
        "Active extension path is missing manifest: {}",
        active_extension_dir.display()
    );

    // Start bootstrap
    let node = std::env::var("TABCTL_NODE_EXEC").unwrap_or_else(|_| "node".to_string());
    let bootstrap_child = Command::new(node)
        .arg(&script)
        .current_dir(&root)
        .env("TABCTL_EXTENSION_DIR", &active_extension_dir)
        .env("TABCTL_HOST_WRAPPER", host_wrapper)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("failed to execute integration bootstrap");

    let pid = bootstrap_child.id();
    BOOTSTRAP_PID.store(pid, Ordering::SeqCst);

    unsafe {
        cleanup::atexit(cleanup_bootstrap);
    }

    // Keep the child handle alive in a guard so it isn't reaped early.
    // ChildGuard won't Drop from a static, but atexit handles cleanup.
    let mut bootstrap = ChildGuard::new(bootstrap_child);

    // Wait for bootstrap readiness
    let mut bootstrap_ready = false;
    let mut last_ping_error = String::new();
    let bootstrap_wait_timeout = Duration::from_secs(180);
    let bootstrap_wait_start = Instant::now();
    while bootstrap_wait_start.elapsed() < bootstrap_wait_timeout {
        if let Some(status) = bootstrap
            .child_mut()
            .try_wait()
            .expect("check bootstrap status")
        {
            panic!("integration bootstrap exited unexpectedly with status {status}");
        }
        match run_tabctl_json_with_timeout(
            &tabctl_bin,
            &root,
            profile_name,
            &config_home,
            &state_home,
            &["ping"],
            Duration::from_secs(45),
        ) {
            Ok(ping) => {
                if ping.get("ok").and_then(Value::as_bool) == Some(true)
                    || (ping.get("ok").is_none() && ping.get("error").is_none())
                {
                    bootstrap_ready = true;
                    break;
                }
                last_ping_error = format!("non-ok ping payload: {ping}");
            }
            Err(error) => last_ping_error = error,
        }
        sleep(Duration::from_millis(500));
    }
    assert!(
        bootstrap_ready,
        "integration bootstrap did not reach ready state before timeout; last ping error: {last_ping_error}"
    );

    // Intentionally leak the ChildGuard so the bootstrap process stays alive
    // for the duration of the test run. The atexit handler cleans up.
    std::mem::forget(bootstrap);

    SharedBrowser {
        tabctl_bin,
        root,
        profile_name: profile_name.to_string(),
        config_home,
        state_home,
    }
}

/// Get the shared browser fixture. Bootstraps Chrome on first call.
pub fn shared_browser() -> &'static SharedBrowser {
    BROWSER.get_or_init(init_browser)
}
