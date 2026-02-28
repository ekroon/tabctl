//! Shared integration-test infrastructure.
//!
//! Provides helpers for running CLI commands and a `SharedBrowser` fixture that
//! boots Chrome + extension exactly once (via `OnceLock`) and exposes a
//! convenient API for browser-backed tests.

#![allow(dead_code, unused_imports)]

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::OnceLock;
use std::thread::sleep;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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

/// Create an isolated sandbox directory for tests. Caller is responsible for cleanup.
pub fn create_sandbox() -> PathBuf {
    let sandbox = std::env::temp_dir().join(format!("tbi-local-{}", now_ms()));
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
    if cfg!(windows) {
        command.env("TABCTL_TRANSPORT", "tcp");
    }
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

    /// Create an isolated test window with the given URLs and optional group.
    /// Returns `(window_id, created_tab_ids)`.
    pub fn create_test_window(&self, urls: &[&str], group: Option<&str>) -> (i64, Vec<i64>) {
        let mut args: Vec<&str> = vec!["open", "--new-window"];
        for url in urls {
            args.push("--url");
            args.push(url);
        }
        if let Some(g) = group {
            args.push("--group");
            args.push(g);
        }
        let open = self.run(&args);
        assert_ok("create_test_window", &open);
        let data = response_data(&open);
        let window_id = data
            .pointer("/windowId")
            .and_then(Value::as_i64)
            .expect("create_test_window: missing windowId");
        let tab_ids = data
            .pointer("/createdTabIds")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_i64).collect())
            .unwrap_or_default();
        (window_id, tab_ids)
    }

    /// Close a test window (best-effort cleanup, errors ignored).
    pub fn close_test_window(&self, window_id: i64) {
        let win_str = window_id.to_string();
        let _ = self.run_result(&["close", "--window", &win_str, "--confirm"]);
    }
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
