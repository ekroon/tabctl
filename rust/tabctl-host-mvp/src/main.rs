use serde::Deserialize;
use serde_json::{json, Value};
use std::env;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const COMPONENT: &str = "host";
const VERSION: &str = env!("CARGO_PKG_VERSION");
const BASE_VERSION: &str = env!("CARGO_PKG_VERSION");
const HISTORY_LIMIT_DEFAULT: usize = 20;
const RETENTION_DAYS: i64 = 30;
const MAX_NATIVE_PAYLOAD_BYTES: usize = 20 * 1024 * 1024;
const FORWARDED_ACTIONS: &[&str] = &[
    "ping",
    "list",
    "analyze",
    "inspect",
    "focus",
    "refresh",
    "open",
    "group-list",
    "group-update",
    "group-ungroup",
    "group-assign",
    "group-gather",
    "move-tab",
    "move-group",
    "merge-window",
    "archive",
    "close",
    "report",
    "screenshot",
    "reload",
];

#[derive(Debug, Deserialize)]
struct Request {
    id: Option<String>,
    action: String,
    #[serde(default)]
    params: Value,
}

fn write_native_message(payload: &Value) -> io::Result<()> {
    let json = serde_json::to_vec(payload)?;
    let len = (json.len() as u32).to_le_bytes();
    let stdout = io::stdout();
    let mut lock = stdout.lock();
    lock.write_all(&len)?;
    lock.write_all(&json)?;
    lock.flush()
}

fn read_native_message(input: &mut dyn Read) -> io::Result<Option<Vec<u8>>> {
    let mut len_buf = [0u8; 4];
    match input.read_exact(&mut len_buf) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(err) => return Err(err),
    }

    let len = u32::from_le_bytes(len_buf) as usize;
    if len > MAX_NATIVE_PAYLOAD_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("native payload exceeds {MAX_NATIVE_PAYLOAD_BYTES} bytes"),
        ));
    }

    let mut payload = vec![0u8; len];
    match input.read_exact(&mut payload) {
        Ok(()) => Ok(Some(payload)),
        Err(err) if err.kind() == io::ErrorKind::UnexpectedEof => Ok(None),
        Err(err) => Err(err),
    }
}

fn ok(action: &str, request_id: Option<&str>, data: Value) -> Value {
    json!({
        "ok": true,
        "action": action,
        "requestId": request_id,
        "component": COMPONENT,
        "version": VERSION,
        "data": data
    })
}

fn err(action: &str, request_id: Option<&str>, message: &str, hint: Option<&str>) -> Value {
    let mut error = json!({ "message": message });
    if let Some(hint_text) = hint {
        error["hint"] = Value::String(hint_text.to_string());
    }
    json!({
        "ok": false,
        "action": action,
        "requestId": request_id,
        "component": COMPONENT,
        "version": VERSION,
        "error": error
    })
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(windows)]
fn default_state_base() -> PathBuf {
    if let Some(path) = env::var_os("LOCALAPPDATA") {
        return PathBuf::from(path);
    }
    if let Some(home) = env::var_os("USERPROFILE") {
        return PathBuf::from(home).join("AppData").join("Local");
    }
    PathBuf::from(".")
}

#[cfg(not(windows))]
fn default_state_base() -> PathBuf {
    if let Some(path) = env::var_os("XDG_STATE_HOME") {
        return PathBuf::from(path);
    }
    if let Some(home) = env::var_os("HOME") {
        return PathBuf::from(home).join(".local").join("state");
    }
    PathBuf::from(".")
}

fn resolve_undo_log_path() -> PathBuf {
    if let Some(path) = env::var_os("TABCTL_UNDO_LOG") {
        return PathBuf::from(path);
    }
    default_state_base().join("tabctl").join("undo.jsonl")
}

fn read_undo_records(path: &Path) -> Vec<Value> {
    let Ok(content) = fs::read_to_string(path) else {
        return Vec::new();
    };
    content
        .lines()
        .filter_map(|line| {
            if line.trim().is_empty() {
                return None;
            }
            serde_json::from_str::<Value>(line).ok()
        })
        .collect()
}

fn filter_by_retention(records: Vec<Value>, now: i64) -> Vec<Value> {
    let cutoff = now - (RETENTION_DAYS * 24 * 60 * 60 * 1000);
    records
        .into_iter()
        .filter(|record| match record.get("createdAt").and_then(Value::as_i64) {
            Some(created_at) => created_at >= cutoff,
            None => true,
        })
        .collect()
}

fn apply_history_limit(records: Vec<Value>, limit_value: Option<&Value>) -> Vec<Value> {
    if records.is_empty() {
        return records;
    }
    let Some(limit) = limit_value.and_then(Value::as_i64) else {
        let start = records.len().saturating_sub(HISTORY_LIMIT_DEFAULT);
        return records[start..].to_vec();
    };

    if limit > 0 {
        let start = records.len().saturating_sub(limit as usize);
        return records[start..].to_vec();
    }

    if limit == 0 {
        return records;
    }

    let skip = usize::try_from(limit.saturating_abs()).unwrap_or(usize::MAX);
    if skip >= records.len() {
        return Vec::new();
    }
    records[skip..].to_vec()
}

fn find_undo_record(records: &[Value], txid: &str) -> Option<Value> {
    for record in records.iter().rev() {
        if record.get("txid").and_then(Value::as_str) == Some(txid) {
            return Some(record.clone());
        }
    }
    None
}

fn find_latest_undo_record(records: &[Value]) -> Option<Value> {
    records.last().cloned()
}

fn is_forwarded_action(action: &str) -> bool {
    FORWARDED_ACTIONS.contains(&action)
}

fn handle_undo(request: &Request, undo_log: &Path) -> Value {
    let request_id = request.id.as_deref();
    let txid = request.params.get("txid").and_then(Value::as_str);
    let latest = request
        .params
        .get("latest")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if txid.is_none() && !latest {
        return err(
            "undo",
            request_id,
            "Missing txid",
            Some("Use tabctl history --json to find a txid, or run tabctl undo --latest"),
        );
    }

    let records = filter_by_retention(read_undo_records(undo_log), now_millis());
    let record = if let Some(known_txid) = txid {
        find_undo_record(&records, known_txid)
    } else {
        find_latest_undo_record(&records)
    };

    let Some(record) = record else {
        return err("undo", request_id, "Undo record not found", None);
    };

    ok(
        "undo",
        request_id,
        json!({
            "status": "not-implemented",
            "txid": record.get("txid").cloned().unwrap_or(Value::Null),
            "latest": latest,
            "record": record,
            "note": "Rust host resolved undo record; extension forwarding is not implemented yet",
            "hostBaseVersion": BASE_VERSION,
            "hostGitSha": Value::Null,
            "hostDirty": false
        }),
    )
}

fn handle_request(request: Request, undo_log: &Path) -> Value {
    let request_id = request.id.as_deref();
    match request.action.as_str() {
        "version" => ok(
            "version",
            request_id,
            json!({
                "component": COMPONENT,
                "version": VERSION,
                "baseVersion": BASE_VERSION,
                "gitSha": Value::Null,
                "dirty": false
            }),
        ),
        "history" => {
            let records = filter_by_retention(read_undo_records(undo_log), now_millis());
            let limited = apply_history_limit(records, request.params.get("limit"));
            ok("history", request_id, Value::Array(limited))
        }
        "undo" => handle_undo(&request, undo_log),
        action if is_forwarded_action(action) => err(
            action,
            request_id,
            "Forwarded action not implemented in Rust MVP",
            Some("Use the default Node host while extension forwarding is incomplete"),
        ),
        action => err(
            action,
            request_id,
            &format!("Unknown action: {action}"),
            Some("See config/protocol/host-protocol.v1.json for supported actions"),
        ),
    }
}

fn main() -> io::Result<()> {
    let undo_log = resolve_undo_log_path();
    let stdin = io::stdin();
    let mut lock = stdin.lock();

    loop {
        let payload = match read_native_message(&mut lock) {
            Ok(Some(payload)) => payload,
            Ok(None) => break,
            Err(_) => {
                write_native_message(&err(
                    "unknown",
                    None,
                    "Invalid native messaging frame",
                    Some("Expected 4-byte little-endian length prefix and bounded JSON payload"),
                ))?;
                break;
            }
        };

        let parsed: Result<Request, _> = serde_json::from_slice(&payload);
        let request = match parsed {
            Ok(req) => req,
            Err(_) => {
                write_native_message(&err(
                    "unknown",
                    None,
                    "Invalid JSON payload",
                    Some("Expected native messaging payload with JSON body"),
                ))?;
                continue;
            }
        };

        let response = handle_request(request, &undo_log);
        write_native_message(&response)?;
    }

    Ok(())
}
