use serde_json::{json, Value};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_NATIVE_PAYLOAD_BYTES: u32 = 20 * 1024 * 1024;

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(prefix: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!("{prefix}-{}-{nanos}", std::process::id()));
        fs::create_dir_all(&path).expect("failed to create temp dir");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

struct HostHarness {
    child: Child,
}

impl HostHarness {
    fn start(undo_log: &Path) -> Self {
        let child = Command::new(env!("CARGO_BIN_EXE_tabctl-host-mvp"))
            .env("TABCTL_UNDO_LOG", undo_log)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("failed to spawn rust host");
        Self { child }
    }

    fn send(&mut self, message: &Value) {
        let payload = serde_json::to_vec(message).expect("serialize request");
        let len = (payload.len() as u32).to_le_bytes();
        let stdin = self.child.stdin.as_mut().expect("child stdin");
        stdin.write_all(&len).expect("write message length");
        stdin.write_all(&payload).expect("write message payload");
        stdin.flush().expect("flush stdin");
    }

    fn send_raw(&mut self, bytes: &[u8]) {
        let stdin = self.child.stdin.as_mut().expect("child stdin");
        stdin.write_all(bytes).expect("write raw bytes");
        stdin.flush().expect("flush stdin");
    }

    fn close_stdin(&mut self) {
        let _ = self.child.stdin.take();
    }

    fn read(&mut self) -> Value {
        let stdout = self.child.stdout.as_mut().expect("child stdout");
        let mut len_buf = [0u8; 4];
        stdout
            .read_exact(&mut len_buf)
            .expect("failed reading response length");
        let len = u32::from_le_bytes(len_buf) as usize;
        let mut payload = vec![0u8; len];
        stdout
            .read_exact(&mut payload)
            .expect("failed reading response payload");
        serde_json::from_slice(&payload).expect("invalid response json")
    }
}

impl Drop for HostHarness {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[test]
fn version_response_uses_host_compat_envelope() {
    let tmp = TempDir::new("tabctl-rust-host-version");
    let undo_log = tmp.path().join("undo.jsonl");
    let mut host = HostHarness::start(&undo_log);

    host.send(&json!({
        "id": "req-version",
        "action": "version",
        "params": {}
    }));
    let response = host.read();

    assert_eq!(response.get("ok"), Some(&Value::Bool(true)));
    assert_eq!(response.get("action"), Some(&Value::String("version".to_string())));
    assert_eq!(
        response.get("requestId"),
        Some(&Value::String("req-version".to_string()))
    );
    assert_eq!(response.get("component"), Some(&Value::String("host".to_string())));
    let version = response
        .get("version")
        .and_then(Value::as_str)
        .expect("top-level version");
    assert!(!version.is_empty());

    let data = response.get("data").expect("missing data");
    assert_eq!(data.get("component"), Some(&Value::String("host".to_string())));
    assert_eq!(data.get("version"), Some(&Value::String(version.to_string())));
    assert_eq!(data.get("baseVersion"), Some(&Value::String(version.to_string())));
    assert!(data.get("dirty").is_some());
    assert!(data.get("gitSha").is_some());
}

#[test]
fn undo_requires_txid_or_latest_with_cli_hint() {
    let tmp = TempDir::new("tabctl-rust-host-undo-hint");
    let undo_log = tmp.path().join("undo.jsonl");
    let mut host = HostHarness::start(&undo_log);

    host.send(&json!({
        "id": "req-undo-missing",
        "action": "undo",
        "params": {}
    }));
    let response = host.read();

    assert_eq!(response.get("ok"), Some(&Value::Bool(false)));
    assert_eq!(response.get("action"), Some(&Value::String("undo".to_string())));
    assert_eq!(response.get("component"), Some(&Value::String("host".to_string())));
    let error = response.get("error").expect("missing error");
    assert_eq!(
        error.get("message"),
        Some(&Value::String("Missing txid".to_string()))
    );
    assert_eq!(
        error.get("hint"),
        Some(&Value::String(
            "Use tabctl history --json to find a txid, or run tabctl undo --latest".to_string()
        ))
    );
}

#[test]
fn history_reads_undo_log_with_retention_and_limit() {
    let tmp = TempDir::new("tabctl-rust-host-history");
    let undo_log = tmp.path().join("undo.jsonl");
    let now = now_millis();
    let old = json!({
        "txid": "old-1",
        "createdAt": now - (40 * 24 * 60 * 60 * 1000),
        "action": "move-tab"
    });
    let keep1 = json!({
        "txid": "keep-1",
        "createdAt": now - (2 * 24 * 60 * 60 * 1000),
        "action": "move-group"
    });
    let keep2 = json!({
        "txid": "keep-2",
        "createdAt": now - (1 * 60 * 60 * 1000),
        "action": "close"
    });
    fs::write(
        &undo_log,
        format!("{old}\n{keep1}\nnot-json\n{keep2}\n"),
    )
    .expect("write undo log");

    let mut host = HostHarness::start(&undo_log);
    host.send(&json!({
        "id": "req-history",
        "action": "history",
        "params": { "limit": 1 }
    }));
    let response = host.read();

    assert_eq!(response.get("ok"), Some(&Value::Bool(true)));
    assert_eq!(response.get("component"), Some(&Value::String("host".to_string())));
    let data = response
        .get("data")
        .and_then(Value::as_array)
        .expect("history data should be array");
    assert_eq!(data.len(), 1);
    assert_eq!(data[0].get("txid"), Some(&Value::String("keep-2".to_string())));
}

#[test]
fn undo_returns_not_found_when_record_missing() {
    let tmp = TempDir::new("tabctl-rust-host-undo-not-found");
    let undo_log = tmp.path().join("undo.jsonl");
    fs::write(
        &undo_log,
        format!(
            "{}\n",
            json!({
                "txid": "known-tx",
                "createdAt": now_millis(),
                "action": "move-tab"
            })
        ),
    )
    .expect("write undo log");
    let mut host = HostHarness::start(&undo_log);

    host.send(&json!({
        "id": "req-undo-not-found",
        "action": "undo",
        "params": { "txid": "missing-tx" }
    }));
    let response = host.read();

    assert_eq!(response.get("ok"), Some(&Value::Bool(false)));
    assert_eq!(response.get("action"), Some(&Value::String("undo".to_string())));
    assert_eq!(
        response
            .get("error")
            .and_then(|e| e.get("message")),
        Some(&Value::String("Undo record not found".to_string()))
    );
}

#[test]
fn forwarded_actions_return_placeholder_error() {
    let tmp = TempDir::new("tabctl-rust-host-forwarded");
    let undo_log = tmp.path().join("undo.jsonl");
    let mut host = HostHarness::start(&undo_log);

    host.send(&json!({
        "id": "req-ping",
        "action": "ping",
        "params": {}
    }));
    let response = host.read();

    assert_eq!(response.get("ok"), Some(&Value::Bool(false)));
    assert_eq!(response.get("action"), Some(&Value::String("ping".to_string())));
    assert_eq!(response.get("component"), Some(&Value::String("host".to_string())));
    assert_eq!(
        response
            .get("error")
            .and_then(|e| e.get("message")),
        Some(&Value::String(
            "Forwarded action not implemented in Rust MVP".to_string()
        ))
    );
}

#[test]
fn oversized_native_frame_returns_protocol_error() {
    let tmp = TempDir::new("tabctl-rust-host-framing");
    let undo_log = tmp.path().join("undo.jsonl");
    let mut host = HostHarness::start(&undo_log);

    let oversized = (MAX_NATIVE_PAYLOAD_BYTES + 1).to_le_bytes();
    host.send_raw(&oversized);
    host.close_stdin();
    let response = host.read();

    assert_eq!(response.get("ok"), Some(&Value::Bool(false)));
    assert_eq!(response.get("action"), Some(&Value::String("unknown".to_string())));
    assert_eq!(response.get("component"), Some(&Value::String("host".to_string())));
    assert_eq!(
        response
            .get("error")
            .and_then(|e| e.get("message")),
        Some(&Value::String("Invalid native messaging frame".to_string()))
    );
}
