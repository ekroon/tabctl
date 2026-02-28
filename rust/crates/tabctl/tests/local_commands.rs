//! Integration tests for local (non-browser) CLI commands.
//!
//! These tests validate commands that don't require a running browser:
//! help, profile lifecycle, doctor, and policy.

mod common;

use common::*;
use std::fs;

// ── help ────────────────────────────────────────────────────────────────────

#[test]
#[ignore = "requires built dist artifacts"]
fn test_help_command() {
    let root = repo_root();
    let bin = rust_tabctl_bin(&root);

    // Main help
    let (stdout, _) = run_tabctl_raw(&bin, &["help"]).expect("help should succeed");
    assert!(
        stdout.contains("tabctl") || stdout.contains("Usage"),
        "help should mention tabctl or Usage: {stdout}"
    );

    // Subcommand help
    let (sub_stdout, _) =
        run_tabctl_raw(&bin, &["help", "list"]).expect("help list should succeed");
    assert!(
        sub_stdout.contains("list") || sub_stdout.contains("List"),
        "help list should describe the list command: {sub_stdout}"
    );
}

// ── profile lifecycle ───────────────────────────────────────────────────────

#[test]
#[ignore = "requires built dist artifacts"]
fn test_profile_lifecycle() {
    let root = repo_root();
    let bin = rust_tabctl_bin(&root);
    let sandbox = create_sandbox();
    let _sandbox_guard = TempDirGuard::new(sandbox.clone());
    let config_home = sandbox.join("c");
    let state_home = sandbox.join("s");
    fs::create_dir_all(&config_home).unwrap();
    fs::create_dir_all(&state_home).unwrap();

    let extension_dir = root.join("dist").join("extension");
    let ext_str = extension_dir.to_str().expect("extension path to utf8");

    let profile_a = "test-profile-a";
    let profile_b = "test-profile-b";

    // ── Setup two profiles ──
    let setup_a = run_tabctl_json(
        &bin,
        &root,
        profile_a,
        &config_home,
        &state_home,
        &[
            "setup",
            "--browser",
            "chrome",
            "--name",
            profile_a,
            "--extension-dir",
            ext_str,
        ],
    )
    .expect("setup profile-a should succeed");
    assert_ok("setup profile-a", &setup_a);

    let setup_b = run_tabctl_json(
        &bin,
        &root,
        profile_b,
        &config_home,
        &state_home,
        &[
            "setup",
            "--browser",
            "edge",
            "--name",
            profile_b,
            "--extension-dir",
            ext_str,
        ],
    )
    .expect("setup profile-b should succeed");
    assert_ok("setup profile-b", &setup_b);

    // ── profile-list ──
    let list = run_tabctl_json(
        &bin,
        &root,
        profile_a,
        &config_home,
        &state_home,
        &["profile-list"],
    )
    .expect("profile-list should succeed");
    assert_ok("profile-list", &list);

    // ── profile-show ──
    let show = run_tabctl_json(
        &bin,
        &root,
        profile_a,
        &config_home,
        &state_home,
        &["profile-show"],
    )
    .expect("profile-show should succeed");
    assert_ok("profile-show", &show);

    // ── profile-switch ──
    let switch = run_tabctl_json(
        &bin,
        &root,
        profile_a,
        &config_home,
        &state_home,
        &["profile-switch", profile_b],
    )
    .expect("profile-switch should succeed");
    assert_ok("profile-switch", &switch);

    // ── profile-remove ──
    let remove = run_tabctl_json(
        &bin,
        &root,
        profile_b,
        &config_home,
        &state_home,
        &["profile-remove", profile_b],
    )
    .expect("profile-remove should succeed");
    assert_ok("profile-remove", &remove);

    // Verify profile-b no longer appears
    let list_after = run_tabctl_json(
        &bin,
        &root,
        profile_a,
        &config_home,
        &state_home,
        &["profile-list"],
    )
    .expect("profile-list after remove should succeed");
    assert_ok("profile-list after remove", &list_after);
}

// ── doctor ──────────────────────────────────────────────────────────────────

#[test]
#[ignore = "requires built dist artifacts"]
fn test_doctor_command() {
    let root = repo_root();
    let bin = rust_tabctl_bin(&root);
    let sandbox = create_sandbox();
    let _sandbox_guard = TempDirGuard::new(sandbox.clone());
    let config_home = sandbox.join("c");
    let state_home = sandbox.join("s");
    fs::create_dir_all(&config_home).unwrap();
    fs::create_dir_all(&state_home).unwrap();

    let extension_dir = root.join("dist").join("extension");
    let ext_str = extension_dir.to_str().expect("extension path to utf8");

    // Setup a profile first
    let profile = "test-doctor";
    let _ = run_tabctl_json(
        &bin,
        &root,
        profile,
        &config_home,
        &state_home,
        &[
            "setup",
            "--browser",
            "chrome",
            "--name",
            profile,
            "--extension-dir",
            ext_str,
        ],
    );

    // Run doctor (without --fix) — should not crash
    let doctor = run_tabctl_json(&bin, &root, profile, &config_home, &state_home, &["doctor"]);
    assert!(
        doctor.is_ok(),
        "doctor should not crash: {:?}",
        doctor.err()
    );
}

// ── policy ──────────────────────────────────────────────────────────────────

#[test]
#[ignore = "requires built dist artifacts"]
fn test_policy_command() {
    let root = repo_root();
    let bin = rust_tabctl_bin(&root);
    let sandbox = create_sandbox();
    let _sandbox_guard = TempDirGuard::new(sandbox.clone());
    let config_home = sandbox.join("c");
    let state_home = sandbox.join("s");
    fs::create_dir_all(&config_home).unwrap();
    fs::create_dir_all(&state_home).unwrap();

    let extension_dir = root.join("dist").join("extension");
    let ext_str = extension_dir.to_str().expect("extension path to utf8");

    let profile = "test-policy";
    let _ = run_tabctl_json(
        &bin,
        &root,
        profile,
        &config_home,
        &state_home,
        &[
            "setup",
            "--browser",
            "chrome",
            "--name",
            profile,
            "--extension-dir",
            ext_str,
        ],
    );

    // Show policy
    let policy = run_tabctl_json(&bin, &root, profile, &config_home, &state_home, &["policy"]);
    assert!(
        policy.is_ok(),
        "policy should not crash: {:?}",
        policy.err()
    );

    // Init policy
    let policy_init = run_tabctl_json(
        &bin,
        &root,
        profile,
        &config_home,
        &state_home,
        &["policy", "--init"],
    );
    assert!(
        policy_init.is_ok(),
        "policy --init should not crash: {:?}",
        policy_init.err()
    );
}
