//! Browser-backed integration smoke test.
//!
//! Rust drives the scenario assertions; a thin Node bootstrap keeps Chrome +
//! extension session management isolated to CDP orchestration.

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

fn repo_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .expect("failed to resolve repository root from CARGO_MANIFEST_DIR")
        .to_path_buf()
}

fn rust_tabctl_bin(root: &Path) -> PathBuf {
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

fn integration_bootstrap_script(root: &Path) -> PathBuf {
    std::env::var("TABCTL_INTEGRATION_BOOTSTRAP")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            root.join("scripts")
                .join("ci")
                .join("integration-bootstrap.js")
        })
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_millis()
}

fn run_tabctl_json(
    tabctl_bin: &Path,
    root: &Path,
    profile: &str,
    config_home: &Path,
    state_home: &Path,
    args: &[&str],
) -> Result<Value, String> {
    let output = Command::new(tabctl_bin)
        .arg("--json")
        .arg("--no-pretty")
        .arg("--profile")
        .arg(profile)
        .args(args)
        .current_dir(root)
        .env("XDG_CONFIG_HOME", config_home)
        .env("XDG_STATE_HOME", state_home)
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
    serde_json::from_str::<Value>(&stdout).map_err(|e| {
        format!(
            "tabctl {:?} returned non-JSON output ({e}). raw stdout: {}",
            args, stdout
        )
    })
}

fn assert_ok(action: &str, payload: &Value) {
    assert_eq!(
        payload.get("ok").and_then(Value::as_bool),
        Some(true),
        "tabctl {action} returned non-ok payload: {payload}"
    );
}

struct ChildGuard {
    child: Child,
}

impl ChildGuard {
    fn new(child: Child) -> Self {
        Self { child }
    }

    fn child_mut(&mut self) -> &mut Child {
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

struct TempDirGuard {
    path: PathBuf,
}

impl TempDirGuard {
    fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[test]
#[ignore = "requires built dist artifacts and Chrome"]
fn real_browser_integration_harness_passes() {
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
        std::env::temp_dir().join(format!("tbi-{}", now_ms()))
    } else {
        PathBuf::from(format!("/tmp/tbi-{}", now_ms()))
    };
    fs::create_dir_all(&sandbox).expect("create test sandbox");
    let _sandbox_guard = TempDirGuard::new(sandbox.clone());
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

    let active_extension_dir = setup
        .pointer("/data/extensionSync/activePath")
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .unwrap_or(extension_dir);
    let host_wrapper = setup
        .pointer("/data/wrapperPath")
        .and_then(Value::as_str)
        .expect("setup payload missing data.wrapperPath");
    assert!(
        active_extension_dir.join("manifest.json").exists(),
        "Active extension path is missing manifest: {}",
        active_extension_dir.display()
    );

    let node = std::env::var("TABCTL_NODE_EXEC").unwrap_or_else(|_| "node".to_string());
    let bootstrap_child = Command::new(node)
        .arg(script)
        .current_dir(&root)
        .env("TABCTL_EXTENSION_DIR", &active_extension_dir)
        .env("TABCTL_HOST_WRAPPER", host_wrapper)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("failed to execute integration bootstrap");
    let mut bootstrap = ChildGuard::new(bootstrap_child);

    let mut bootstrap_ready = false;
    let mut last_ping_error = String::new();
    for _ in 0..60 {
        if let Some(status) = bootstrap
            .child_mut()
            .try_wait()
            .expect("check bootstrap status")
        {
            panic!("integration bootstrap exited unexpectedly with status {status}");
        }
        match run_tabctl_json(
            &tabctl_bin,
            &root,
            profile_name,
            &config_home,
            &state_home,
            &["ping"],
        ) {
            Ok(ping) => {
                if ping.get("ok").and_then(Value::as_bool) == Some(true) {
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

    let ping = run_tabctl_json(
        &tabctl_bin,
        &root,
        profile_name,
        &config_home,
        &state_home,
        &["ping"],
    )
    .expect("ping command should succeed");
    assert_ok("ping", &ping);

    let version = run_tabctl_json(
        &tabctl_bin,
        &root,
        profile_name,
        &config_home,
        &state_home,
        &["version"],
    )
    .expect("version command should succeed");
    assert_ok("version", &version);

    let list_all = run_tabctl_json(
        &tabctl_bin,
        &root,
        profile_name,
        &config_home,
        &state_home,
        &["list", "--all"],
    )
    .expect("list --all should succeed");
    assert_ok("list --all", &list_all);

    let test_group = format!("TEST-Rust-Integration-{}", now_ms());
    let open = run_tabctl_json(
        &tabctl_bin,
        &root,
        profile_name,
        &config_home,
        &state_home,
        &[
            "open",
            "--new-window",
            "--url",
            "https://example.com",
            "--url",
            "https://example.org",
            "--group",
            test_group.as_str(),
        ],
    )
    .expect("open should succeed");
    assert_ok("open", &open);

    sleep(Duration::from_secs(2));

    let list_last_focused = run_tabctl_json(
        &tabctl_bin,
        &root,
        profile_name,
        &config_home,
        &state_home,
        &["list", "--window", "last-focused"],
    )
    .expect("list --window last-focused should succeed");
    assert_ok("list --window last-focused", &list_last_focused);

    let groups = run_tabctl_json(
        &tabctl_bin,
        &root,
        profile_name,
        &config_home,
        &state_home,
        &["group-list", "--window", "last-focused"],
    )
    .expect("group-list should succeed");
    assert_ok("group-list", &groups);

    let close_group = run_tabctl_json(
        &tabctl_bin,
        &root,
        profile_name,
        &config_home,
        &state_home,
        &[
            "close",
            "--group",
            test_group.as_str(),
            "--window",
            "last-focused",
            "--confirm",
        ],
    )
    .expect("close should succeed");
    assert_ok("close", &close_group);

    let undo = run_tabctl_json(
        &tabctl_bin,
        &root,
        profile_name,
        &config_home,
        &state_home,
        &["undo", "--latest"],
    )
    .expect("undo should succeed");
    assert_ok("undo --latest", &undo);
}
