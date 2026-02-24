//! Browser-backed integration smoke test.
//!
//! This wraps the existing real-browser integration harness and runs it from
//! `cargo test` so integration can be driven from Rust/CI entrypoints.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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

#[test]
#[ignore = "requires built dist artifacts and Chrome"]
fn real_browser_integration_harness_passes() {
    let root = repo_root();
    let script = root
        .join("dist")
        .join("scripts")
        .join("integration-test.js");
    assert!(
        script.exists(),
        "integration harness not found at {} (run `npm run build` first)",
        script.display()
    );

    let tabctl_bin = rust_tabctl_bin(&root);
    assert!(
        tabctl_bin.exists(),
        "Rust tabctl binary not found at {} (run `npm run build` first)",
        tabctl_bin.display()
    );

    let node = std::env::var("TABCTL_NODE_EXEC").unwrap_or_else(|_| "node".to_string());
    let status = Command::new(node)
        .arg(script)
        .current_dir(&root)
        .env("TABCTL_CLI_IMPL", "rust-force")
        .env("TABCTL_RUST_CLI_BIN", tabctl_bin)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .expect("failed to execute browser integration harness");

    assert!(
        status.success(),
        "browser integration harness failed with status {status}"
    );
}
