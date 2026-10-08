use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::path::Path;

use super::protocol::now_ms;

pub(super) const RETENTION_DAYS: u64 = 30;

fn parse_journal(file_path: &Path, bytes: &[u8]) -> Result<Vec<Map<String, Value>>, String> {
    let content = std::str::from_utf8(bytes).map_err(|error| {
        journal_error(
            file_path,
            format!("invalid UTF-8 at byte {}", error.valid_up_to()),
        )
    })?;
    let mut records: Vec<Map<String, Value>> = Vec::new();
    let mut positions = HashMap::new();
    for (index, line) in content.split('\n').enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let value: Value = serde_json::from_str(line).map_err(|error| {
            journal_error(
                file_path,
                format!("invalid record on line {}: {error}", index + 1),
            )
        })?;
        let record = value.as_object().cloned().ok_or_else(|| {
            journal_error(
                file_path,
                format!("line {} is not a JSON object", index + 1),
            )
        })?;
        let txid = record.get("txid").and_then(Value::as_str);
        if let Some(pos) = txid.and_then(|txid| positions.get(txid)).copied() {
            records[pos] = record;
        } else {
            if let Some(txid) = txid {
                positions.insert(txid.to_owned(), records.len());
            }
            records.push(record);
        }
    }
    Ok(records)
}

fn journal_error(file_path: &Path, detail: impl std::fmt::Display) -> String {
    format!("Undo journal {}: {detail}. Preserve and inspect the journal; repair it before retrying recovery or mutations.", file_path.display())
}

fn lock_journal(file_path: &Path, file: &fs::File, exclusive: bool) -> Result<(), String> {
    extern "C" {
        fn flock(fd: std::ffi::c_int, operation: std::ffi::c_int) -> std::ffi::c_int;
    }
    let operation = if exclusive { 2 } else { 1 }; // LOCK_EX / LOCK_SH on macOS.
    loop {
        // The borrowed File owns a valid descriptor; close releases the lock.
        if unsafe { flock(file.as_raw_fd(), operation) } == 0 {
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::Interrupted {
            return Err(journal_error(file_path, error));
        }
    }
}

pub(super) fn read_undo_records_checked(
    file_path: &Path,
) -> Result<Vec<Map<String, Value>>, String> {
    let mut file = match fs::File::open(file_path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(journal_error(file_path, error)),
    };
    lock_journal(file_path, &file, false)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|error| journal_error(file_path, error))?;
    parse_journal(file_path, &bytes)
}

#[cfg(test)]
pub(super) fn read_undo_records(file_path: &Path) -> Vec<Map<String, Value>> {
    read_undo_records_checked(file_path).expect("test undo journal must be readable and valid")
}

pub(super) fn append_undo_record(
    file_path: &Path,
    record: &Map<String, Value>,
) -> Result<bool, String> {
    if record_contains_incognito(&Value::Object(record.clone())) {
        read_undo_records_checked(file_path)?;
        return Ok(false);
    }
    if let Some(parent) = file_path.parent() {
        fs::create_dir_all(parent).map_err(|error| journal_error(file_path, error))?;
    }
    let mut sanitized = Value::Object(record.clone());
    strip_incognito_markers(&mut sanitized);
    let serialized =
        serde_json::to_vec(&sanitized).map_err(|error| journal_error(file_path, error))?;
    let mut file = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(file_path)
        .map_err(|error| journal_error(file_path, error))?;
    lock_journal(file_path, &file, true)?;
    let mut existing = Vec::new();
    file.read_to_end(&mut existing)
        .map_err(|error| journal_error(file_path, error))?;
    parse_journal(file_path, &existing)?;
    let mut frame = Vec::with_capacity(serialized.len() + 2);
    if existing.last().is_some_and(|byte| *byte != b'\n') {
        frame.push(b'\n');
    }
    frame.extend_from_slice(&serialized);
    frame.push(b'\n');
    // Buffer the entire frame and synchronize it. This is not a crash-atomic
    // append: a torn tail must fail validation before subsequent mutations.
    file.write_all(&frame)
        .and_then(|_| file.sync_all())
        .map_err(|error| journal_error(file_path, error))?;
    Ok(true)
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

pub(super) fn find_undo_record(
    file_path: &Path,
    txid: &str,
) -> Result<Option<Map<String, Value>>, String> {
    let records = filter_by_retention(read_undo_records_checked(file_path)?, RETENTION_DAYS);
    Ok(records
        .into_iter()
        .rev()
        .find(|record| record.get("txid").and_then(|v| v.as_str()) == Some(txid)))
}

#[cfg(test)]
pub(super) fn find_latest_undo_record(
    file_path: &Path,
) -> Result<Option<Map<String, Value>>, String> {
    find_latest_undo_record_excluding(file_path, &HashSet::new())
}

pub(super) fn find_latest_undo_record_excluding(
    file_path: &Path,
    excluded: &HashSet<String>,
) -> Result<Option<Map<String, Value>>, String> {
    let records = filter_by_retention(read_undo_records_checked(file_path)?, RETENTION_DAYS);
    Ok(records.into_iter().rev().find(|record| {
        is_pending_undo(record)
            && !record
                .get("txid")
                .and_then(Value::as_str)
                .is_some_and(|id| excluded.contains(id))
    }))
}

pub(super) fn unresolved_creation(record: &Map<String, Value>) -> bool {
    matches!(
        record
            .get("inFlight")
            .and_then(|step| step.get("action"))
            .and_then(Value::as_str),
        Some("p:tab-create" | "p:window-create")
    )
}

pub(super) fn is_pending_undo(record: &Map<String, Value>) -> bool {
    !matches!(
        record.get("status").and_then(Value::as_str),
        Some("undone" | "undoing" | "undo_uncertain" | "recovery_uncertain")
    ) && !unresolved_creation(record)
        && record.get("undo").is_some_and(|undo| !undo.is_null())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_undo_path() -> std::path::PathBuf {
        let unique = crate::host_impl::protocol::create_id("undo-test");
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/undo-tests")
            .join(unique)
            .join("undo.jsonl")
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
        assert!(!append_undo_record(&path, record.as_object().unwrap()).unwrap());
        assert!(read_undo_records(&path).is_empty());
    }

    #[test]
    fn checkpoints_replace_history_and_latest_skips_consumed_records() {
        let path = temp_undo_path();
        let record = serde_json::json!({"txid":"tx-1","status":"in_progress","undo":{"action":"close","tabs":[]}});
        append_undo_record(&path, record.as_object().unwrap()).unwrap();
        let mut done = record.as_object().unwrap().clone();
        done.insert("status".into(), Value::String("undone".into()));
        append_undo_record(&path, &done).unwrap();
        assert_eq!(read_undo_records(&path).len(), 1);
        assert!(find_latest_undo_record(&path).unwrap().is_none());
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn append_preserves_valid_unterminated_final_record() {
        let path = temp_undo_path();
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let original = br#"{"txid":"first","undo":{"action":"restore","tabs":[]}}"#;
        fs::write(&path, original).unwrap();
        let next = serde_json::json!({"txid":"second","undo":{"action":"restore","tabs":[]}});
        append_undo_record(&path, next.as_object().unwrap()).unwrap();
        let bytes = fs::read(&path).unwrap();
        assert!(bytes.starts_with(original));
        assert_eq!(bytes[original.len()], b'\n');
        assert_eq!(read_undo_records(&path).len(), 2);
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn invalid_journal_is_unchanged_when_append_is_rejected() {
        for bytes in [
            b"{".as_slice(),
            b"{\"txid\":\"valid\"}\n{\"txid\":\"cut-off".as_slice(),
            b"{\"txid\":\"valid\"}\n\xff".as_slice(),
            b"{\"txid\":\"valid\"}\n{\"txid\":\"cut-\xf0\x9f".as_slice(),
            b"[]\n".as_slice(),
        ] {
            let path = temp_undo_path();
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, bytes).unwrap();
            let next = serde_json::json!({"txid":"next"});
            let error = append_undo_record(&path, next.as_object().unwrap()).unwrap_err();
            assert!(error.contains("journal") && error.contains(&path.display().to_string()));
            assert_eq!(fs::read(&path).unwrap(), bytes);
            assert!(read_undo_records_checked(&path).is_err());
            let private = serde_json::json!({"txid":"private","undo":{"incognito":true}});
            assert!(append_undo_record(&path, private.as_object().unwrap()).is_err());
            assert_eq!(fs::read(&path).unwrap(), bytes);
            fs::remove_dir_all(path.parent().unwrap()).unwrap();
        }
    }

    #[test]
    fn missing_and_empty_journals_are_valid_but_io_errors_are_not_empty_history() {
        let path = temp_undo_path();
        assert!(read_undo_records_checked(&path).unwrap().is_empty());
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, []).unwrap();
        assert!(read_undo_records_checked(&path).unwrap().is_empty());
        let error = read_undo_records_checked(path.parent().unwrap()).unwrap_err();
        assert!(
            error.contains("journal")
                && error.contains(&path.parent().unwrap().display().to_string())
        );
        let record = serde_json::json!({"txid":"first"});
        append_undo_record(&path, record.as_object().unwrap()).unwrap();
        assert_eq!(read_undo_records_checked(&path).unwrap().len(), 1);
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn concurrent_appends_preserve_complete_frames() {
        let path = temp_undo_path();
        let writers: Vec<_> = (0..4).map(|writer| {
            let path = path.clone();
            std::thread::spawn(move || {
                for sequence in 0..8 {
                    let record = serde_json::json!({"txid":format!("{writer}-{sequence}"),"text":"Unicode 🦀"});
                    append_undo_record(&path, record.as_object().unwrap()).unwrap();
                }
            })
        }).collect();
        for writer in writers {
            writer.join().unwrap();
        }
        assert_eq!(read_undo_records_checked(&path).unwrap().len(), 32);
        assert_eq!(fs::read_to_string(&path).unwrap().lines().count(), 32);
        fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }
}
