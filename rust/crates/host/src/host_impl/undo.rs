use serde_json::{Map, Value};
use std::fs;
use std::io::Write;
use std::path::Path;

use super::protocol::now_ms;

pub(super) const RETENTION_DAYS: u64 = 30;

pub(super) fn read_undo_records(file_path: &Path) -> Vec<Map<String, Value>> {
    let Ok(content) = fs::read_to_string(file_path) else {
        return Vec::new();
    };

    content
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|v| v.as_object().cloned())
        .collect()
}

pub(super) fn append_undo_record(file_path: &Path, record: &Map<String, Value>) {
    if record_contains_incognito(&Value::Object(record.clone())) {
        return;
    }
    if let Some(parent) = file_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let mut sanitized = Value::Object(record.clone());
    strip_incognito_markers(&mut sanitized);
    if let Ok(serialized) = serde_json::to_string(&sanitized) {
        let _ = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(file_path)
            .and_then(|mut f| writeln!(f, "{serialized}"));
    }
}

fn record_contains_incognito(value: &Value) -> bool {
    match value {
        Value::Object(map) => map.iter().any(|(key, child)| {
            (key == "incognito" && child.as_bool() == Some(true))
                || record_contains_incognito(child)
        }),
        Value::Array(items) => items.iter().any(record_contains_incognito),
        _ => false,
    }
}

fn strip_incognito_markers(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.remove("incognito");
            for child in map.values_mut() {
                strip_incognito_markers(child);
            }
        }
        Value::Array(items) => {
            for child in items {
                strip_incognito_markers(child);
            }
        }
        _ => {}
    }
}

pub(super) fn filter_by_retention(
    records: Vec<Map<String, Value>>,
    retention_days: u64,
) -> Vec<Map<String, Value>> {
    let cutoff = now_ms().saturating_sub(retention_days * 24 * 60 * 60 * 1000);
    records
        .into_iter()
        .filter(|record| {
            record
                .get("createdAt")
                .and_then(|v| v.as_u64())
                .map(|created_at| created_at >= cutoff)
                .unwrap_or(true)
        })
        .collect()
}

pub(super) fn find_undo_record(file_path: &Path, txid: &str) -> Option<Map<String, Value>> {
    let records = filter_by_retention(read_undo_records(file_path), RETENTION_DAYS);
    records
        .into_iter()
        .rev()
        .find(|record| record.get("txid").and_then(|v| v.as_str()) == Some(txid))
}

pub(super) fn find_latest_undo_record(file_path: &Path) -> Option<Map<String, Value>> {
    let records = filter_by_retention(read_undo_records(file_path), RETENTION_DAYS);
    records.into_iter().last()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_undo_path() -> std::path::PathBuf {
        let unique = format!(
            "tabctl-undo-test-{}-{}",
            std::process::id(),
            crate::host_impl::protocol::now_ms()
        );
        std::env::temp_dir().join(unique).join("undo.jsonl")
    }

    #[test]
    fn skips_records_marked_incognito() {
        let path = temp_undo_path();
        let record = serde_json::json!({
            "txid": "tx-1",
            "createdAt": 1,
            "action": "close",
            "summary": {"closedTabs": 1},
            "undo": {
                "action": "close",
                "incognito": true,
                "tabs": [{"url": "https://secret.example"}]
            }
        });
        append_undo_record(&path, record.as_object().unwrap());
        assert!(read_undo_records(&path).is_empty());
    }
}
