use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(3)
        .expect("repository root")
        .to_path_buf()
}

pub fn rust_tabctl_bin(root: &Path) -> PathBuf {
    root.join("rust/target/debug/tabctl")
}

pub fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_millis()
}

static SANDBOX_COUNTER: AtomicU32 = AtomicU32::new(0);

pub fn create_sandbox() -> PathBuf {
    let seq = SANDBOX_COUNTER.fetch_add(1, Ordering::Relaxed);
    let root = std::env::var("TABCTL_TEST_TMP_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp/tctl-it"));
    let sandbox = root.join(format!("i{}-{}-{seq}", now_ms(), std::process::id()));
    fs::create_dir_all(&sandbox).expect("create test sandbox");
    sandbox
}

pub fn assert_ok(action: &str, payload: &Value) {
    assert_ne!(
        payload.get("ok").and_then(Value::as_bool),
        Some(false),
        "{action}: {payload}"
    );
    assert!(payload.get("error").is_none(), "{action}: {payload}");
    assert!(
        payload
            .get("errors")
            .and_then(Value::as_array)
            .map_or(true, Vec::is_empty),
        "{action}: {payload}"
    );
}

pub fn response_data(payload: &Value) -> &Value {
    payload.get("data").unwrap_or(payload)
}

pub fn gql_string(value: &str) -> String {
    serde_json::to_string(value).expect("serialize GraphQL literal")
}

pub struct TempDirGuard(PathBuf);

impl TempDirGuard {
    pub fn new(path: PathBuf) -> Self {
        Self(path)
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        if let Err(error) = fs::remove_dir_all(&self.0) {
            if error.kind() != std::io::ErrorKind::NotFound {
                eprintln!(
                    "Failed to remove test sandbox {}: {error}",
                    self.0.display()
                );
            }
        }
    }
}
