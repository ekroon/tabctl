//! Browser-backed integration smoke test.
//!
//! Rust drives the scenario assertions; a thin Node bootstrap keeps Chrome +
//! extension session management isolated to CDP orchestration.

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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

fn run_tabctl_json_with_timeout(
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
        if let Some(_status) = child
            .try_wait()
            .map_err(|e| format!("failed to poll tabctl {:?}: {e}", args))?
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

fn run_tabctl_json(
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
    let open_window_id = open
        .pointer("/data/windowId")
        .and_then(Value::as_i64)
        .expect("open payload missing data.windowId");
    let open_window_id_arg = open_window_id.to_string();
    sleep(Duration::from_secs(2));

    let open_reuse_group = run_tabctl_json(
        &tabctl_bin,
        &root,
        profile_name,
        &config_home,
        &state_home,
        &[
            "open",
            "--window",
            open_window_id_arg.as_str(),
            "--group",
            test_group.as_str(),
            "--color",
            "blue",
            "--url",
            "https://example.com",
            "--url",
            "https://example.net",
        ],
    )
    .expect("open into existing group should succeed");
    assert_ok("open into existing group", &open_reuse_group);
    assert_eq!(
        open_reuse_group
            .pointer("/data/summary/createdTabs")
            .and_then(Value::as_u64),
        Some(1),
        "expected duplicate URL to be skipped and one tab to be created: {open_reuse_group}"
    );
    assert_eq!(
        open_reuse_group
            .pointer("/data/summary/skippedUrls")
            .and_then(Value::as_u64),
        Some(1),
        "expected one skipped duplicate URL: {open_reuse_group}"
    );

    sleep(Duration::from_secs(2));

    let list_last_focused = run_tabctl_json(
        &tabctl_bin,
        &root,
        profile_name,
        &config_home,
        &state_home,
        &["list", "--window", open_window_id_arg.as_str()],
    )
    .expect("list --window <open window> should succeed");
    assert_ok("list --window <open window>", &list_last_focused);
    let windows = list_last_focused
        .pointer("/data/windows")
        .and_then(Value::as_array)
        .expect("list payload missing data.windows");
    let mut example_net_group_title: Option<String> = None;
    let mut example_net_ungrouped = false;
    for window in windows {
        if let Some(tabs) = window.get("tabs").and_then(Value::as_array) {
            for tab in tabs {
                let url = tab.get("url").and_then(Value::as_str).unwrap_or_default();
                if url.starts_with("https://example.net") {
                    if tab.get("groupId").and_then(Value::as_i64) == Some(-1) {
                        example_net_ungrouped = true;
                    }
                    example_net_group_title = tab
                        .get("groupTitle")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned);
                }
            }
        }
    }
    assert_eq!(
        example_net_group_title.as_deref(),
        Some(test_group.as_str()),
        "example.net tab should be in the target group: {list_last_focused}"
    );
    assert!(
        !example_net_ungrouped,
        "example.net tab must not be left ungrouped: {list_last_focused}"
    );

    let busy_marker = now_ms();
    let anchor_group = format!("TEST-Rust-Anchor-{}", busy_marker);
    let noise_url_a = format!("https://example.edu/?noise={busy_marker}");
    let noise_url_b = format!("https://example.gov/?noise={busy_marker}");
    let anchor_url = format!("https://example.org/?anchor={busy_marker}");
    let move_url_a = format!("https://example.dev/?movea={busy_marker}");
    let move_url_b = format!("https://example.app/?moveb={busy_marker}");

    let open_noise = run_tabctl_json(
        &tabctl_bin,
        &root,
        profile_name,
        &config_home,
        &state_home,
        &[
            "open",
            "--window",
            open_window_id_arg.as_str(),
            "--url",
            noise_url_a.as_str(),
            "--url",
            noise_url_b.as_str(),
        ],
    )
    .expect("open ungrouped noise tabs should succeed");
    assert_ok("open ungrouped noise tabs", &open_noise);

    let open_anchor_group = run_tabctl_json(
        &tabctl_bin,
        &root,
        profile_name,
        &config_home,
        &state_home,
        &[
            "open",
            "--window",
            open_window_id_arg.as_str(),
            "--group",
            anchor_group.as_str(),
            "--url",
            anchor_url.as_str(),
        ],
    )
    .expect("open anchor group should succeed");
    assert_ok("open anchor group", &open_anchor_group);

    let open_busy_reuse = run_tabctl_json(
        &tabctl_bin,
        &root,
        profile_name,
        &config_home,
        &state_home,
        &[
            "open",
            "--window",
            open_window_id_arg.as_str(),
            "--group",
            test_group.as_str(),
            "--url",
            move_url_a.as_str(),
            "--url",
            move_url_b.as_str(),
        ],
    )
    .expect("open into busy existing group should succeed");
    assert_ok("open into busy existing group", &open_busy_reuse);
    assert_eq!(
        open_busy_reuse
            .pointer("/data/summary/createdTabs")
            .and_then(Value::as_u64),
        Some(2),
        "expected two created tabs in busy reuse scenario: {open_busy_reuse}"
    );
    let busy_created_tab_ids: Vec<i64> = open_busy_reuse
        .pointer("/data/created")
        .and_then(Value::as_array)
        .expect("open busy reuse payload missing data.created")
        .iter()
        .filter_map(|entry| entry.get("tabId").and_then(Value::as_i64))
        .collect();
    assert_eq!(
        busy_created_tab_ids.len(),
        2,
        "expected two created tab ids in busy reuse scenario: {open_busy_reuse}"
    );

    let assert_move_tabs_grouped = |payload: &Value, phase: &str| {
        let windows = payload
            .pointer("/data/windows")
            .and_then(Value::as_array)
            .expect("list payload missing data.windows");
        let mut found: Vec<(i64, Option<String>, i64)> = Vec::new();
        for window in windows {
            if let Some(tabs) = window.get("tabs").and_then(Value::as_array) {
                for tab in tabs {
                    if let Some(tab_id) = tab.get("tabId").and_then(Value::as_i64) {
                        if busy_created_tab_ids.contains(&tab_id) {
                            found.push((
                                tab.get("groupId").and_then(Value::as_i64).unwrap_or(-1),
                                tab.get("groupTitle")
                                    .and_then(Value::as_str)
                                    .map(ToOwned::to_owned),
                                tab_id,
                            ));
                        }
                    }
                }
            }
        }
        assert_eq!(
            found.len(),
            busy_created_tab_ids.len(),
            "expected to find all created move tabs during {phase}: {payload}"
        );
        for (group_id, group_title, tab_id) in found {
            assert_ne!(
                group_id, -1,
                "tab {tab_id} must remain grouped during {phase}: {payload}"
            );
            assert_eq!(
                group_title.as_deref(),
                Some(test_group.as_str()),
                "tab {tab_id} must stay in group {test_group} during {phase}: {payload}"
            );
        }
    };

    let list_busy_open = run_tabctl_json(
        &tabctl_bin,
        &root,
        profile_name,
        &config_home,
        &state_home,
        &["list", "--window", open_window_id_arg.as_str()],
    )
    .expect("list busy window after open should succeed");
    assert_ok("list busy window after open", &list_busy_open);
    assert_move_tabs_grouped(&list_busy_open, "busy-open-immediate");

    sleep(Duration::from_secs(2));
    let list_busy_open_delayed = run_tabctl_json(
        &tabctl_bin,
        &root,
        profile_name,
        &config_home,
        &state_home,
        &["list", "--window", open_window_id_arg.as_str()],
    )
    .expect("list busy window after delayed open should succeed");
    assert_ok("list busy window after delayed open", &list_busy_open_delayed);
    assert_move_tabs_grouped(&list_busy_open_delayed, "busy-open-delayed");

    let move_group = run_tabctl_json(
        &tabctl_bin,
        &root,
        profile_name,
        &config_home,
        &state_home,
        &[
            "move-group",
            "--window",
            open_window_id_arg.as_str(),
            "--group",
            test_group.as_str(),
            "--after-group",
            anchor_group.as_str(),
        ],
    )
    .expect("move-group should succeed");
    assert_ok("move-group", &move_group);

    let list_after_move = run_tabctl_json(
        &tabctl_bin,
        &root,
        profile_name,
        &config_home,
        &state_home,
        &["list", "--window", open_window_id_arg.as_str()],
    )
    .expect("list after move-group should succeed");
    assert_ok("list after move-group", &list_after_move);
    assert_move_tabs_grouped(&list_after_move, "move-group-immediate");

    sleep(Duration::from_secs(2));
    let list_after_move_delayed = run_tabctl_json(
        &tabctl_bin,
        &root,
        profile_name,
        &config_home,
        &state_home,
        &["list", "--window", open_window_id_arg.as_str()],
    )
    .expect("list after delayed move-group should succeed");
    assert_ok("list after delayed move-group", &list_after_move_delayed);
    assert_move_tabs_grouped(&list_after_move_delayed, "move-group-delayed");

    let groups = run_tabctl_json(
        &tabctl_bin,
        &root,
        profile_name,
        &config_home,
        &state_home,
        &["group-list", "--window", open_window_id_arg.as_str()],
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
            open_window_id_arg.as_str(),
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
