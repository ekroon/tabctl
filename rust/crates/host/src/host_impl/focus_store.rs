use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use std::path::Path;
use tabctl_shared::normalize_url;

/// Open (or create) the focus SQLite database at the given path.
pub(super) fn open_focus_db(db_path: &Path) -> Result<Connection, String> {
    if let Some(parent) = db_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let conn = Connection::open(db_path).map_err(|e| format!("Failed to open focus DB: {e}"))?;

    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS tab_access (
            normalized_url TEXT NOT NULL,
            raw_url TEXT NOT NULL,
            title TEXT,
            last_accessed_at INTEGER NOT NULL,
            profile TEXT,
            updated_at INTEGER NOT NULL,
            PRIMARY KEY (normalized_url, profile)
        );
        CREATE INDEX IF NOT EXISTS idx_tab_access_ts
            ON tab_access(last_accessed_at DESC);",
    )
    .map_err(|e| format!("Failed to create focus DB schema: {e}"))?;

    Ok(conn)
}

/// Upsert a tab's access timestamp into the focus store.
/// Only updates if the new timestamp is more recent.
pub(super) fn upsert_access(
    conn: &Connection,
    url: &str,
    title: Option<&str>,
    last_accessed_at: i64,
    profile: Option<&str>,
) {
    let normalized = normalize_url(url);
    let profile_str = profile.unwrap_or("");
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);

    let _ = conn.execute(
        "INSERT INTO tab_access (normalized_url, raw_url, title, last_accessed_at, profile, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(normalized_url, profile) DO UPDATE SET
             raw_url = excluded.raw_url,
             title = COALESCE(excluded.title, tab_access.title),
             last_accessed_at = MAX(tab_access.last_accessed_at, excluded.last_accessed_at),
             updated_at = excluded.updated_at",
        params![normalized, url, title, last_accessed_at, profile_str, now_ms],
    );
}

/// Look up the most recent access timestamp for a URL across all profiles.
pub(super) fn lookup_access(conn: &Connection, url: &str) -> Option<i64> {
    let normalized = normalize_url(url);
    conn.query_row(
        "SELECT MAX(last_accessed_at) FROM tab_access WHERE normalized_url = ?1",
        params![normalized],
        |row| row.get(0),
    )
    .optional()
    .ok()
    .flatten()
}

/// Process a snapshot: upsert all tabs with timestamps, then enrich tabs
/// missing timestamps from the store. Mutates the snapshot in place.
pub(super) fn enrich_snapshot(conn: &Connection, snapshot: &mut Value, profile: Option<&str>) {
    let Some(windows) = snapshot.get_mut("windows").and_then(Value::as_array_mut) else {
        return;
    };

    for window in windows.iter_mut() {
        let Some(tabs) = window.get_mut("tabs").and_then(Value::as_array_mut) else {
            continue;
        };

        for tab in tabs.iter_mut() {
            let url = tab.get("url").and_then(Value::as_str).unwrap_or("");
            if url.is_empty() {
                continue;
            }

            let title = tab.get("title").and_then(Value::as_str);
            let ts = tab.get("lastAccessedAt").and_then(Value::as_i64);

            if let Some(ts) = ts {
                // Tab has a timestamp — store it
                upsert_access(conn, url, title, ts, profile);
            } else {
                // Tab has no timestamp — try to enrich from store
                if let Some(stored_ts) = lookup_access(conn, url) {
                    tab.as_object_mut().map(|obj| {
                        obj.insert(
                            "lastAccessedAt".to_string(),
                            Value::Number(stored_ts.into()),
                        )
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn in_memory_db() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE tab_access (
                normalized_url TEXT NOT NULL,
                raw_url TEXT NOT NULL,
                title TEXT,
                last_accessed_at INTEGER NOT NULL,
                profile TEXT,
                updated_at INTEGER NOT NULL,
                PRIMARY KEY (normalized_url, profile)
            );
            CREATE INDEX idx_tab_access_ts ON tab_access(last_accessed_at DESC);",
        )
        .unwrap();
        conn
    }

    #[test]
    fn upsert_and_lookup() {
        let conn = in_memory_db();
        upsert_access(&conn, "https://example.com", Some("Example"), 1000, None);
        assert_eq!(lookup_access(&conn, "https://example.com"), Some(1000));
    }

    #[test]
    fn upsert_keeps_higher_timestamp() {
        let conn = in_memory_db();
        upsert_access(&conn, "https://example.com", Some("Old"), 1000, None);
        upsert_access(&conn, "https://example.com", Some("New"), 500, None);
        assert_eq!(lookup_access(&conn, "https://example.com"), Some(1000));
    }

    #[test]
    fn lookup_normalizes_url() {
        let conn = in_memory_db();
        upsert_access(&conn, "https://www.example.com/page#frag", None, 2000, None);
        assert_eq!(lookup_access(&conn, "http://example.com/page"), Some(2000));
    }

    #[test]
    fn cross_profile_lookup() {
        let conn = in_memory_db();
        upsert_access(&conn, "https://example.com", None, 1000, Some("edge"));
        upsert_access(&conn, "https://example.com", None, 2000, Some("chrome"));
        // MAX across profiles
        assert_eq!(lookup_access(&conn, "https://example.com"), Some(2000));
    }

    #[test]
    fn enrich_snapshot_fills_missing_timestamps() {
        let conn = in_memory_db();
        upsert_access(&conn, "https://a.com", Some("A"), 5000, None);

        let mut snapshot = serde_json::json!({
            "windows": [{
                "windowId": 1,
                "tabs": [
                    {"tabId": 1, "url": "https://a.com", "title": "A"},
                    {"tabId": 2, "url": "https://b.com", "title": "B", "lastAccessedAt": 3000}
                ]
            }]
        });

        enrich_snapshot(&conn, &mut snapshot, None);

        let tabs = snapshot["windows"][0]["tabs"].as_array().unwrap();
        // Tab 1 was enriched from store
        assert_eq!(tabs[0]["lastAccessedAt"], 5000);
        // Tab 2 kept its own timestamp and was stored
        assert_eq!(tabs[1]["lastAccessedAt"], 3000);
        assert_eq!(lookup_access(&conn, "https://b.com"), Some(3000));
    }
}
