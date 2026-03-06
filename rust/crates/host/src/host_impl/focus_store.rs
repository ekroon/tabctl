use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, fs, path::Path};
use tabctl_shared::normalize_url;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FocusEntry {
    normalized_url: String,
    raw_url: String,
    title: Option<String>,
    last_accessed_at: i64,
    profile: String,
    updated_at: i64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct FocusStore {
    #[serde(default)]
    entries: BTreeMap<String, FocusEntry>,
}

fn store_key(normalized_url: &str, profile: Option<&str>) -> String {
    format!("{}\u{1f}{normalized_url}", profile.unwrap_or(""))
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn load_store(store_path: &Path) -> Result<FocusStore, String> {
    let Ok(content) = fs::read_to_string(store_path) else {
        return Ok(FocusStore::default());
    };

    if content.trim().is_empty() {
        return Ok(FocusStore::default());
    }

    serde_json::from_str(&content).map_err(|e| format!("Failed to parse focus store: {e}"))
}

fn save_store(store_path: &Path, store: &FocusStore) -> Result<(), String> {
    if let Some(parent) = store_path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("Failed to create focus store dir: {e}"))?;
    }

    let serialized =
        serde_json::to_vec(store).map_err(|e| format!("Failed to serialize focus store: {e}"))?;
    fs::write(store_path, serialized).map_err(|e| format!("Failed to write focus store: {e}"))
}

fn upsert_access(
    store: &mut FocusStore,
    url: &str,
    title: Option<&str>,
    last_accessed_at: i64,
    profile: Option<&str>,
) {
    let normalized = normalize_url(url);
    let profile_str = profile.unwrap_or("");
    let key = store_key(&normalized, profile);
    let updated_at = now_ms();

    match store.entries.get_mut(&key) {
        Some(existing) => {
            existing.raw_url = url.to_string();
            if let Some(title) = title {
                existing.title = Some(title.to_string());
            }
            existing.last_accessed_at = existing.last_accessed_at.max(last_accessed_at);
            existing.updated_at = updated_at;
        }
        None => {
            store.entries.insert(
                key,
                FocusEntry {
                    normalized_url: normalized,
                    raw_url: url.to_string(),
                    title: title.map(ToOwned::to_owned),
                    last_accessed_at,
                    profile: profile_str.to_string(),
                    updated_at,
                },
            );
        }
    }
}

fn lookup_access(store: &FocusStore, url: &str) -> Option<i64> {
    let normalized = normalize_url(url);
    store
        .entries
        .values()
        .filter(|entry| entry.normalized_url == normalized)
        .map(|entry| entry.last_accessed_at)
        .max()
}

pub(super) fn enrich_snapshot(
    store_path: &Path,
    snapshot: &mut Value,
    profile: Option<&str>,
) -> Result<(), String> {
    let mut store = load_store(store_path)?;
    let Some(windows) = snapshot.get_mut("windows").and_then(Value::as_array_mut) else {
        return Ok(());
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
                upsert_access(&mut store, url, title, ts, profile);
                continue;
            }

            if let Some(stored_ts) = lookup_access(&store, url) {
                if let Some(obj) = tab.as_object_mut() {
                    obj.insert(
                        "lastAccessedAt".to_string(),
                        Value::Number(stored_ts.into()),
                    );
                }
            }
        }
    }

    save_store(store_path, &store)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempStorePath(std::path::PathBuf);

    impl TempStorePath {
        fn new() -> Self {
            let unique = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            Self(std::env::temp_dir().join(format!("tabctl-focus-store-{unique}.json")))
        }

        fn as_path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempStorePath {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }

    #[test]
    fn upsert_and_lookup() {
        let mut store = FocusStore::default();
        upsert_access(
            &mut store,
            "https://example.com",
            Some("Example"),
            1000,
            None,
        );
        assert_eq!(lookup_access(&store, "https://example.com"), Some(1000));
    }

    #[test]
    fn upsert_keeps_higher_timestamp() {
        let mut store = FocusStore::default();
        upsert_access(&mut store, "https://example.com", Some("Old"), 1000, None);
        upsert_access(&mut store, "https://example.com", Some("New"), 500, None);
        assert_eq!(lookup_access(&store, "https://example.com"), Some(1000));
    }

    #[test]
    fn lookup_normalizes_url() {
        let mut store = FocusStore::default();
        upsert_access(
            &mut store,
            "https://www.example.com/page#frag",
            None,
            2000,
            None,
        );
        assert_eq!(lookup_access(&store, "http://example.com/page"), Some(2000));
    }

    #[test]
    fn cross_profile_lookup() {
        let mut store = FocusStore::default();
        upsert_access(&mut store, "https://example.com", None, 1000, Some("edge"));
        upsert_access(
            &mut store,
            "https://example.com",
            None,
            2000,
            Some("chrome"),
        );
        assert_eq!(lookup_access(&store, "https://example.com"), Some(2000));
    }

    #[test]
    fn enrich_snapshot_fills_missing_timestamps() {
        let path = TempStorePath::new();
        let mut store = FocusStore::default();
        upsert_access(&mut store, "https://a.com", Some("A"), 5000, None);
        save_store(path.as_path(), &store).unwrap();

        let mut snapshot = serde_json::json!({
            "windows": [{
                "windowId": 1,
                "tabs": [
                    {"tabId": 1, "url": "https://a.com", "title": "A"},
                    {"tabId": 2, "url": "https://b.com", "title": "B", "lastAccessedAt": 3000}
                ]
            }]
        });

        enrich_snapshot(path.as_path(), &mut snapshot, None).unwrap();
        let persisted = load_store(path.as_path()).unwrap();

        let tabs = snapshot["windows"][0]["tabs"].as_array().unwrap();
        assert_eq!(tabs[0]["lastAccessedAt"], 5000);
        assert_eq!(tabs[1]["lastAccessedAt"], 3000);
        assert_eq!(lookup_access(&persisted, "https://b.com"), Some(3000));
    }
}
