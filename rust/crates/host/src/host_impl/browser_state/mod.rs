use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;
use tabctl_shared::normalize_url;

const DEFAULT_HISTORY_LIMIT: usize = 20;
const DEFAULT_EVENT_LIMIT: usize = 50;

#[derive(Debug, Clone)]
struct PreviousGroup {
    logical_group_id: String,
    title: String,
    color: String,
    tab_urls: Vec<String>,
}

#[derive(Debug, Clone)]
struct GroupObservation {
    browser_group_id: i64,
    browser_window_id: i64,
    window_ordinal: i64,
    title: String,
    color: String,
    collapsed: Option<bool>,
    tab_count: i64,
    tab_urls: Vec<String>,
    logical_group_id: String,
}

#[derive(Debug, Clone)]
struct WindowObservation {
    browser_window_id: i64,
    window_ordinal: i64,
    focused: bool,
    state: Option<String>,
    tab_count: i64,
    logical_window_id: String,
}

#[derive(Debug, Clone)]
struct SnapshotMeta {
    snapshot_id: i64,
    snapshot_hash: String,
}

fn profile_name(profile: Option<&str>) -> &str {
    profile.unwrap_or("")
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn hash_string(input: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(input.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn open_db(db_path: &Path) -> Result<Connection, String> {
    if let Some(parent) = db_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create browser-state db dir: {e}"))?;
        }
    }
    let conn = Connection::open(db_path).map_err(|e| format!("Failed to open sqlite db: {e}"))?;
    conn.execute_batch(
        r#"
        PRAGMA foreign_keys = ON;
        CREATE TABLE IF NOT EXISTS browser_state_snapshots (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            profile_name TEXT NOT NULL,
            recorded_at INTEGER NOT NULL,
            reason TEXT NOT NULL,
            event_count INTEGER NOT NULL,
            event_kinds_json TEXT NOT NULL,
            previous_snapshot_id INTEGER,
            snapshot_hash TEXT NOT NULL,
            snapshot_json TEXT NOT NULL,
            window_count INTEGER NOT NULL,
            group_count INTEGER NOT NULL,
            tab_count INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_browser_state_snapshots_profile_recorded
            ON browser_state_snapshots(profile_name, recorded_at DESC, id DESC);

        CREATE TABLE IF NOT EXISTS browser_state_windows (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            snapshot_id INTEGER NOT NULL REFERENCES browser_state_snapshots(id) ON DELETE CASCADE,
            profile_name TEXT NOT NULL,
            logical_window_id TEXT NOT NULL,
            browser_window_id INTEGER NOT NULL,
            window_ordinal INTEGER NOT NULL,
            focused INTEGER NOT NULL,
            state TEXT,
            tab_count INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_browser_state_windows_snapshot
            ON browser_state_windows(snapshot_id, window_ordinal);

        CREATE TABLE IF NOT EXISTS browser_state_groups (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            snapshot_id INTEGER NOT NULL REFERENCES browser_state_snapshots(id) ON DELETE CASCADE,
            profile_name TEXT NOT NULL,
            logical_group_id TEXT NOT NULL,
            logical_window_id TEXT NOT NULL,
            browser_group_id INTEGER NOT NULL,
            browser_window_id INTEGER NOT NULL,
            window_ordinal INTEGER NOT NULL,
            title TEXT NOT NULL,
            color TEXT NOT NULL,
            collapsed INTEGER,
            tab_count INTEGER NOT NULL,
            tab_urls_json TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_browser_state_groups_snapshot
            ON browser_state_groups(snapshot_id, window_ordinal, browser_group_id);
        CREATE INDEX IF NOT EXISTS idx_browser_state_groups_logical
            ON browser_state_groups(profile_name, logical_group_id, snapshot_id DESC);
        CREATE INDEX IF NOT EXISTS idx_browser_state_groups_title
            ON browser_state_groups(profile_name, title, snapshot_id DESC);

        CREATE TABLE IF NOT EXISTS browser_state_events (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            profile_name TEXT NOT NULL,
            recorded_at INTEGER NOT NULL,
            reason TEXT NOT NULL,
            before_snapshot_id INTEGER,
            after_snapshot_id INTEGER NOT NULL REFERENCES browser_state_snapshots(id) ON DELETE CASCADE,
            kind TEXT NOT NULL,
            browser_window_id INTEGER,
            browser_group_id INTEGER,
            browser_tab_id INTEGER,
            browser_tab_url TEXT,
            payload_json TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_browser_state_events_profile_recorded
            ON browser_state_events(profile_name, recorded_at DESC, id DESC);
        CREATE INDEX IF NOT EXISTS idx_browser_state_events_after_snapshot
            ON browser_state_events(after_snapshot_id, id);
        "#,
    )
    .map_err(|e| format!("Failed to initialize browser-state schema: {e}"))?;
    ensure_event_tab_url_column(&conn)?;
    Ok(conn)
}

fn ensure_event_tab_url_column(conn: &Connection) -> Result<(), String> {
    let has_column = conn
        .query_row(
            "SELECT 1
             FROM pragma_table_info('browser_state_events')
             WHERE name = 'browser_tab_url'
             LIMIT 1",
            [],
            |_| Ok(()),
        )
        .optional()
        .map_err(|e| format!("Failed to inspect browser-state event schema: {e}"))?
        .is_some();

    if !has_column {
        conn.execute(
            "ALTER TABLE browser_state_events ADD COLUMN browser_tab_url TEXT",
            [],
        )
        .map_err(|e| format!("Failed to migrate browser-state event schema: {e}"))?;
    }

    conn.execute(
        "CREATE INDEX IF NOT EXISTS idx_browser_state_events_tab_url
         ON browser_state_events(profile_name, browser_tab_url, after_snapshot_id DESC)",
        [],
    )
    .map_err(|e| format!("Failed to index browser-state event URLs: {e}"))?;
    Ok(())
}

fn previous_snapshot_meta(
    conn: &Connection,
    profile: &str,
) -> Result<Option<SnapshotMeta>, String> {
    conn.query_row(
        "SELECT id, snapshot_hash FROM browser_state_snapshots WHERE profile_name = ?1 ORDER BY id DESC LIMIT 1",
        params![profile],
        |row| {
            Ok(SnapshotMeta {
                snapshot_id: row.get(0)?,
                snapshot_hash: row.get(1)?,
            })
        },
    )
    .optional()
    .map_err(|e| format!("Failed to load latest browser-state snapshot: {e}"))
}

fn snapshot_by_id(conn: &Connection, snapshot_id: i64) -> Result<Value, String> {
    let snapshot_json: String = conn
        .query_row(
            "SELECT snapshot_json FROM browser_state_snapshots WHERE id = ?1",
            params![snapshot_id],
            |row| row.get(0),
        )
        .map_err(|e| format!("Failed to load browser-state snapshot {snapshot_id}: {e}"))?;
    serde_json::from_str::<Value>(&snapshot_json)
        .map_err(|e| format!("Invalid stored snapshot JSON for {snapshot_id}: {e}"))
}

fn previous_groups(
    conn: &Connection,
    profile: &str,
    snapshot_id: i64,
) -> Result<Vec<PreviousGroup>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT logical_group_id, title, color, tab_urls_json
             FROM browser_state_groups
             WHERE profile_name = ?1 AND snapshot_id = ?2
             ORDER BY id ASC",
        )
        .map_err(|e| format!("Failed to prepare previous group query: {e}"))?;
    let rows = stmt
        .query_map(params![profile, snapshot_id], |row| {
            let tab_urls_json: String = row.get(3)?;
            let tab_urls = serde_json::from_str::<Vec<String>>(&tab_urls_json).unwrap_or_default();
            Ok(PreviousGroup {
                logical_group_id: row.get(0)?,
                title: row.get(1)?,
                color: row.get(2)?,
                tab_urls,
            })
        })
        .map_err(|e| format!("Failed to query previous groups: {e}"))?;
    let mut groups = Vec::new();
    for row in rows {
        groups.push(row.map_err(|e| format!("Failed to load previous group row: {e}"))?);
    }
    Ok(groups)
}

fn overlap_score(a: &[String], b: &[String]) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let a_set: HashSet<&str> = a.iter().map(String::as_str).collect();
    let b_set: HashSet<&str> = b.iter().map(String::as_str).collect();
    let intersection = a_set.intersection(&b_set).count() as f64;
    let max_len = a_set.len().max(b_set.len()) as f64;
    intersection / max_len
}

fn fresh_group_id(group: &GroupObservation, recorded_at: i64) -> String {
    let seed = format!(
        "{}|{}|{}|{}|{}|{}",
        recorded_at,
        group.window_ordinal,
        group.browser_group_id,
        group.title,
        group.color,
        group.tab_urls.join("|")
    );
    format!("grp-{}", &hash_string(&seed)[..16])
}

fn extract_group_observations(snapshot: &Value) -> Vec<GroupObservation> {
    let Some(windows) = snapshot.get("windows").and_then(Value::as_array) else {
        return Vec::new();
    };

    let mut groups = Vec::new();
    for (window_ordinal, window) in windows.iter().enumerate() {
        let browser_window_id = window.get("windowId").and_then(Value::as_i64).unwrap_or(-1);
        let tabs = window
            .get("tabs")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let group_defs = window
            .get("groups")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        for group in group_defs {
            let Some(browser_group_id) = group.get("groupId").and_then(Value::as_i64) else {
                continue;
            };
            let mut tab_urls = BTreeSet::new();
            let mut tab_count = 0_i64;
            for tab in &tabs {
                if tab.get("groupId").and_then(Value::as_i64) == Some(browser_group_id) {
                    tab_count += 1;
                    let url = tab.get("url").and_then(Value::as_str).unwrap_or("");
                    if !url.is_empty() {
                        tab_urls.insert(normalize_url(url));
                    }
                }
            }
            groups.push(GroupObservation {
                browser_group_id,
                browser_window_id,
                window_ordinal: window_ordinal as i64,
                title: group
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                color: group
                    .get("color")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                collapsed: group.get("collapsed").and_then(Value::as_bool),
                tab_count,
                tab_urls: tab_urls.into_iter().collect(),
                logical_group_id: String::new(),
            });
        }
    }

    groups
}

fn assign_logical_group_ids(
    groups: &mut [GroupObservation],
    previous: &[PreviousGroup],
    recorded_at: i64,
) {
    let mut used_previous = HashSet::new();
    for group in groups.iter_mut() {
        let mut best_match: Option<(usize, f64)> = None;
        for (idx, prev) in previous.iter().enumerate() {
            if used_previous.contains(&idx) {
                continue;
            }
            let overlap = overlap_score(&group.tab_urls, &prev.tab_urls);
            if overlap <= 0.0 {
                continue;
            }
            let mut score = overlap;
            if group.title == prev.title {
                score += 0.25;
            }
            if group.color == prev.color {
                score += 0.1;
            }
            if score > best_match.map(|(_, existing)| existing).unwrap_or(f64::MIN) {
                best_match = Some((idx, score));
            }
        }

        if let Some((idx, score)) = best_match {
            if score >= 0.5 {
                used_previous.insert(idx);
                group.logical_group_id = previous[idx].logical_group_id.clone();
                continue;
            }
        }

        group.logical_group_id = fresh_group_id(group, recorded_at);
    }
}

fn build_window_observations(
    snapshot: &Value,
    groups: &[GroupObservation],
) -> Vec<WindowObservation> {
    let Some(windows) = snapshot.get("windows").and_then(Value::as_array) else {
        return Vec::new();
    };

    let mut grouped_by_window: HashMap<i64, Vec<&GroupObservation>> = HashMap::new();
    for group in groups {
        grouped_by_window
            .entry(group.browser_window_id)
            .or_default()
            .push(group);
    }

    let mut observations = Vec::new();
    for (window_ordinal, window) in windows.iter().enumerate() {
        let browser_window_id = window.get("windowId").and_then(Value::as_i64).unwrap_or(-1);
        let tabs = window
            .get("tabs")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let mut signature_parts = grouped_by_window
            .get(&browser_window_id)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .map(|group| format!("g:{}:{}", group.logical_group_id, group.tab_urls.join(",")))
            .collect::<Vec<_>>();

        let mut ungrouped_urls = BTreeSet::new();
        for tab in &tabs {
            if tab.get("groupId").and_then(Value::as_i64).unwrap_or(-1) == -1 {
                let url = tab.get("url").and_then(Value::as_str).unwrap_or("");
                if !url.is_empty() {
                    ungrouped_urls.insert(normalize_url(url));
                }
            }
        }
        signature_parts.push(format!(
            "u:{}",
            ungrouped_urls.into_iter().collect::<Vec<_>>().join(",")
        ));
        let logical_window_id = format!("win-{}", &hash_string(&signature_parts.join("|"))[..16]);
        observations.push(WindowObservation {
            browser_window_id,
            window_ordinal: window_ordinal as i64,
            focused: window
                .get("focused")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            state: window
                .get("state")
                .and_then(Value::as_str)
                .map(String::from),
            tab_count: tabs.len() as i64,
            logical_window_id,
        });
    }

    observations
}

fn snapshot_counts(snapshot: &Value) -> (i64, i64, i64) {
    let Some(windows) = snapshot.get("windows").and_then(Value::as_array) else {
        return (0, 0, 0);
    };
    let window_count = windows.len() as i64;
    let mut group_count = 0_i64;
    let mut tab_count = 0_i64;
    for window in windows {
        group_count += window
            .get("groups")
            .and_then(Value::as_array)
            .map(|groups| groups.len() as i64)
            .unwrap_or(0);
        tab_count += window
            .get("tabs")
            .and_then(Value::as_array)
            .map(|tabs| tabs.len() as i64)
            .unwrap_or(0);
    }
    (window_count, group_count, tab_count)
}

fn sanitized_snapshot(snapshot: &Value) -> Value {
    let Some(windows) = snapshot.get("windows").and_then(Value::as_array) else {
        return snapshot.clone();
    };

    let filtered_windows = windows
        .iter()
        .filter(|window| window.get("incognito").and_then(Value::as_bool) != Some(true))
        .map(|window| {
            let mut cloned = window.clone();
            if let Some(window_obj) = cloned.as_object_mut() {
                window_obj.remove("incognito");
                if let Some(tabs) = window_obj.get_mut("tabs").and_then(Value::as_array_mut) {
                    for tab in tabs.iter_mut() {
                        if let Some(tab_obj) = tab.as_object_mut() {
                            tab_obj.remove("incognito");
                        }
                    }
                }
            }
            cloned
        })
        .collect::<Vec<_>>();

    let mut sanitized = snapshot.clone();
    if let Some(snapshot_obj) = sanitized.as_object_mut() {
        snapshot_obj.insert("windows".to_string(), Value::Array(filtered_windows));
    }
    sanitized
}

fn incognito_ids(snapshot: &Value) -> (HashSet<i64>, HashSet<i64>, HashSet<i64>) {
    let windows = snapshot
        .get("windows")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut window_ids = HashSet::new();
    let mut tab_ids = HashSet::new();
    let mut group_ids = HashSet::new();
    for window in windows {
        if window.get("incognito").and_then(Value::as_bool) != Some(true) {
            continue;
        }
        if let Some(window_id) = window.get("windowId").and_then(Value::as_i64) {
            window_ids.insert(window_id);
        }
        if let Some(tabs) = window.get("tabs").and_then(Value::as_array) {
            for tab in tabs {
                if let Some(tab_id) = tab.get("tabId").and_then(Value::as_i64) {
                    tab_ids.insert(tab_id);
                }
            }
        }
        if let Some(groups) = window.get("groups").and_then(Value::as_array) {
            for group in groups {
                if let Some(group_id) = group.get("groupId").and_then(Value::as_i64) {
                    group_ids.insert(group_id);
                }
            }
        }
    }
    (window_ids, group_ids, tab_ids)
}

fn sanitized_events(snapshot: &Value, events: &[Value]) -> Vec<Value> {
    let (incognito_window_ids, incognito_group_ids, incognito_tab_ids) = incognito_ids(snapshot);
    events
        .iter()
        .filter_map(|event| {
            let is_incognito = event.get("incognito").and_then(Value::as_bool) == Some(true)
                || event
                    .get("windowId")
                    .and_then(Value::as_i64)
                    .map(|window_id| incognito_window_ids.contains(&window_id))
                    .unwrap_or(false)
                || event
                    .get("tabId")
                    .and_then(Value::as_i64)
                    .map(|tab_id| incognito_tab_ids.contains(&tab_id))
                    .unwrap_or(false)
                || event
                    .get("groupId")
                    .and_then(Value::as_i64)
                    .map(|group_id| incognito_group_ids.contains(&group_id))
                    .unwrap_or(false);
            if is_incognito {
                return None;
            }
            let mut sanitized = event.clone();
            if let Some(event_obj) = sanitized.as_object_mut() {
                event_obj.remove("incognito");
            }
            Some(sanitized)
        })
        .collect()
}

fn tab_urls_by_id(snapshot: &Value) -> HashMap<i64, String> {
    let mut urls = HashMap::new();
    for window in snapshot
        .get("windows")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        for tab in window
            .get("tabs")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(tab_id) = tab.get("tabId").and_then(Value::as_i64) else {
                continue;
            };
            let Some(url) = tab.get("url").and_then(Value::as_str) else {
                continue;
            };
            if !url.is_empty() {
                urls.insert(tab_id, normalize_url(url));
            }
        }
    }
    urls
}

fn normalized_event_tab_url(
    event: &Value,
    current_tab_urls: &HashMap<i64, String>,
    previous_tab_urls: &HashMap<i64, String>,
) -> Option<String> {
    let explicit_url = event
        .get("url")
        .and_then(Value::as_str)
        .or_else(|| event.get("pendingUrl").and_then(Value::as_str))
        .or_else(|| {
            event
                .get("changeInfo")
                .and_then(|change| change.get("url"))
                .and_then(Value::as_str)
        });
    if let Some(url) = explicit_url {
        return (!url.is_empty()).then(|| normalize_url(url));
    }

    let tab_id = event.get("tabId").and_then(Value::as_i64)?;
    current_tab_urls
        .get(&tab_id)
        .or_else(|| previous_tab_urls.get(&tab_id))
        .cloned()
}

fn normalized_event_prune_urls(
    event: &Value,
    current_tab_urls: &HashMap<i64, String>,
    previous_tab_urls: &HashMap<i64, String>,
) -> HashSet<String> {
    let mut urls = HashSet::new();
    if let Some(url) = normalized_event_tab_url(event, current_tab_urls, previous_tab_urls) {
        urls.insert(url);
    }
    if let Some(tab_id) = event.get("tabId").and_then(Value::as_i64) {
        if let Some(url) = current_tab_urls.get(&tab_id) {
            urls.insert(url.clone());
        }
        if let Some(url) = previous_tab_urls.get(&tab_id) {
            urls.insert(url.clone());
        }
    }
    urls
}

fn prune_replaced_event_snapshots(
    tx: &rusqlite::Transaction<'_>,
    profile: &str,
    current_snapshot_id: i64,
    event_urls: &HashSet<String>,
    open_urls: &HashSet<String>,
) -> Result<usize, String> {
    let mut protected_snapshot_ids = HashSet::new();
    protected_snapshot_ids.insert(current_snapshot_id);
    for open_url in open_urls {
        let latest = tx
            .query_row(
                "SELECT MAX(after_snapshot_id)
                 FROM browser_state_events
                 WHERE profile_name = ?1 AND browser_tab_url = ?2",
                params![profile, open_url],
                |row| row.get::<_, Option<i64>>(0),
            )
            .map_err(|e| format!("Failed to find latest open browser-state snapshot: {e}"))?;
        if let Some(snapshot_id) = latest {
            protected_snapshot_ids.insert(snapshot_id);
        }
    }

    let mut pruned = 0;
    for event_url in event_urls {
        let url_is_open = open_urls.contains(event_url);
        let mut stmt = tx
            .prepare(
                "SELECT DISTINCT old.after_snapshot_id
                 FROM browser_state_events old
                 WHERE old.profile_name = ?1
                   AND old.browser_tab_url = ?2
                   AND (?3 = 0 OR old.after_snapshot_id != ?4)",
            )
            .map_err(|e| format!("Failed to prepare replaced browser-state snapshot query: {e}"))?;
        let rows = stmt
            .query_map(
                params![
                    profile,
                    event_url,
                    if url_is_open { 1 } else { 0 },
                    current_snapshot_id,
                ],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|e| format!("Failed to query replaced browser-state snapshots: {e}"))?;
        let mut candidates = Vec::new();
        for row in rows {
            let snapshot_id =
                row.map_err(|e| format!("Failed to read replaced browser-state snapshot: {e}"))?;
            if !protected_snapshot_ids.contains(&snapshot_id) {
                candidates.push(snapshot_id);
            }
        }
        drop(stmt);

        for snapshot_id in candidates {
            pruned += tx
                .execute(
                    "DELETE FROM browser_state_snapshots WHERE profile_name = ?1 AND id = ?2",
                    params![profile, snapshot_id],
                )
                .map_err(|e| format!("Failed to prune replaced browser-state snapshot: {e}"))?;
        }
    }
    Ok(pruned)
}

pub(super) fn ingest_sync(
    db_path: &Path,
    profile: Option<&str>,
    payload: &Value,
) -> Result<Value, String> {
    let profile = profile_name(profile);
    let snapshot = payload
        .get("snapshot")
        .cloned()
        .or_else(|| payload.get("windows").map(|_| payload.clone()))
        .ok_or_else(|| "browser-state-sync missing snapshot".to_string())?;
    let events = payload
        .get("events")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let events = sanitized_events(&snapshot, &events);
    let snapshot = sanitized_snapshot(&snapshot);
    let reason = payload
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or(if events.is_empty() {
            "snapshot"
        } else {
            "event"
        });
    let recorded_at = payload
        .get("recordedAt")
        .and_then(Value::as_i64)
        .unwrap_or_else(now_ms);
    let event_kinds = events
        .iter()
        .filter_map(|event| event.get("kind").and_then(Value::as_str).map(String::from))
        .collect::<Vec<_>>();
    let snapshot_json = serde_json::to_string(&snapshot)
        .map_err(|e| format!("Failed to serialize snapshot: {e}"))?;
    let snapshot_hash = hash_string(&snapshot_json);

    let mut conn = open_db(db_path)?;
    let previous = previous_snapshot_meta(&conn, profile)?;
    if previous
        .as_ref()
        .is_some_and(|prev| prev.snapshot_hash == snapshot_hash && events.is_empty())
    {
        return Ok(serde_json::json!({
            "ingested": false,
            "reason": reason,
            "eventCount": 0,
            "snapshotHash": snapshot_hash,
        }));
    }

    let previous_snapshot = previous
        .as_ref()
        .map(|prev| snapshot_by_id(&conn, prev.snapshot_id))
        .transpose()?;
    let current_tab_urls = tab_urls_by_id(&snapshot);
    let previous_tab_urls = previous_snapshot
        .as_ref()
        .map(tab_urls_by_id)
        .unwrap_or_default();
    let open_urls = current_tab_urls.values().cloned().collect::<HashSet<_>>();
    let event_tab_urls = events
        .iter()
        .map(|event| normalized_event_tab_url(event, &current_tab_urls, &previous_tab_urls))
        .collect::<Vec<_>>();
    let event_prune_urls = events
        .iter()
        .flat_map(|event| normalized_event_prune_urls(event, &current_tab_urls, &previous_tab_urls))
        .collect::<HashSet<_>>();

    let previous_groups_list = match previous.as_ref() {
        Some(prev) => previous_groups(&conn, profile, prev.snapshot_id)?,
        None => Vec::new(),
    };

    let mut groups = extract_group_observations(&snapshot);
    assign_logical_group_ids(&mut groups, &previous_groups_list, recorded_at);
    let windows = build_window_observations(&snapshot, &groups);
    let logical_window_by_browser = windows
        .iter()
        .map(|window| (window.browser_window_id, window.logical_window_id.clone()))
        .collect::<HashMap<_, _>>();
    let (window_count, group_count, tab_count) = snapshot_counts(&snapshot);

    let tx = conn
        .transaction()
        .map_err(|e| format!("Failed to start browser-state transaction: {e}"))?;
    tx.execute(
        "INSERT INTO browser_state_snapshots (
            profile_name, recorded_at, reason, event_count, event_kinds_json,
            previous_snapshot_id, snapshot_hash, snapshot_json, window_count, group_count, tab_count
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            profile,
            recorded_at,
            reason,
            events.len() as i64,
            serde_json::to_string(&event_kinds).unwrap_or_else(|_| "[]".to_string()),
            previous.as_ref().map(|prev| prev.snapshot_id),
            snapshot_hash,
            snapshot_json,
            window_count,
            group_count,
            tab_count,
        ],
    )
    .map_err(|e| format!("Failed to insert browser-state snapshot: {e}"))?;
    let snapshot_id = tx.last_insert_rowid();

    for window in &windows {
        tx.execute(
            "INSERT INTO browser_state_windows (
                snapshot_id, profile_name, logical_window_id, browser_window_id,
                window_ordinal, focused, state, tab_count
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                snapshot_id,
                profile,
                window.logical_window_id,
                window.browser_window_id,
                window.window_ordinal,
                if window.focused { 1 } else { 0 },
                window.state,
                window.tab_count,
            ],
        )
        .map_err(|e| format!("Failed to insert browser-state window: {e}"))?;
    }

    for group in &groups {
        tx.execute(
            "INSERT INTO browser_state_groups (
                snapshot_id, profile_name, logical_group_id, logical_window_id,
                browser_group_id, browser_window_id, window_ordinal, title, color,
                collapsed, tab_count, tab_urls_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                snapshot_id,
                profile,
                group.logical_group_id,
                logical_window_by_browser
                    .get(&group.browser_window_id)
                    .cloned()
                    .unwrap_or_default(),
                group.browser_group_id,
                group.browser_window_id,
                group.window_ordinal,
                group.title,
                group.color,
                group.collapsed.map(|value| if value { 1 } else { 0 }),
                group.tab_count,
                serde_json::to_string(&group.tab_urls).unwrap_or_else(|_| "[]".to_string()),
            ],
        )
        .map_err(|e| format!("Failed to insert browser-state group: {e}"))?;
    }

    for (index, event) in events.iter().enumerate() {
        let kind = event
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("browser.event")
            .to_string();
        let payload_json =
            serde_json::to_string(event).map_err(|e| format!("Failed to serialize event: {e}"))?;
        tx.execute(
            "INSERT INTO browser_state_events (
                profile_name, recorded_at, reason, before_snapshot_id, after_snapshot_id,
                kind, browser_window_id, browser_group_id, browser_tab_id, browser_tab_url,
                payload_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            params![
                profile,
                event
                    .get("occurredAt")
                    .and_then(Value::as_i64)
                    .unwrap_or(recorded_at),
                reason,
                previous.as_ref().map(|prev| prev.snapshot_id),
                snapshot_id,
                kind,
                event.get("windowId").and_then(Value::as_i64),
                event.get("groupId").and_then(Value::as_i64),
                event.get("tabId").and_then(Value::as_i64),
                event_tab_urls.get(index).and_then(Option::as_deref),
                payload_json,
            ],
        )
        .map_err(|e| format!("Failed to insert browser-state event: {e}"))?;
    }

    let pruned_snapshots =
        prune_replaced_event_snapshots(&tx, profile, snapshot_id, &event_prune_urls, &open_urls)?;

    tx.commit()
        .map_err(|e| format!("Failed to commit browser-state transaction: {e}"))?;

    Ok(serde_json::json!({
        "ingested": true,
        "snapshotId": snapshot_id,
        "reason": reason,
        "eventCount": events.len(),
        "eventKinds": event_kinds,
        "windowCount": window_count,
        "groupCount": group_count,
        "tabCount": tab_count,
        "previousSnapshotId": previous.map(|prev| prev.snapshot_id),
        "prunedSnapshots": pruned_snapshots,
    }))
}

pub(super) fn list_history(
    db_path: &Path,
    profile: Option<&str>,
    limit: Option<usize>,
) -> Result<Value, String> {
    let profile = profile_name(profile);
    let conn = open_db(db_path)?;
    let limit = limit.unwrap_or(DEFAULT_HISTORY_LIMIT) as i64;
    let mut stmt = conn
        .prepare(
            "SELECT id, recorded_at, reason, event_count, event_kinds_json,
                    previous_snapshot_id, window_count, group_count, tab_count
             FROM browser_state_snapshots
             WHERE profile_name = ?1
             ORDER BY id DESC
             LIMIT ?2",
        )
        .map_err(|e| format!("Failed to prepare browser-state history query: {e}"))?;
    let rows = stmt
        .query_map(params![profile, limit], |row| {
            let event_kinds_json: String = row.get(4)?;
            let event_kinds =
                serde_json::from_str::<Vec<String>>(&event_kinds_json).unwrap_or_default();
            Ok(serde_json::json!({
                "snapshotId": row.get::<_, i64>(0)?,
                "recordedAt": row.get::<_, i64>(1)?,
                "reason": row.get::<_, String>(2)?,
                "eventCount": row.get::<_, i64>(3)?,
                "eventKinds": event_kinds,
                "previousSnapshotId": row.get::<_, Option<i64>>(5)?,
                "windowCount": row.get::<_, i64>(6)?,
                "groupCount": row.get::<_, i64>(7)?,
                "tabCount": row.get::<_, i64>(8)?,
            }))
        })
        .map_err(|e| format!("Failed to run browser-state history query: {e}"))?;
    let mut items = Vec::new();
    for row in rows {
        items.push(row.map_err(|e| format!("Failed to read browser-state history row: {e}"))?);
    }
    Ok(Value::Array(items))
}

pub(super) fn latest_snapshot(db_path: &Path, profile: Option<&str>) -> Result<Value, String> {
    let profile = profile_name(profile);
    let conn = open_db(db_path)?;
    let row = conn
        .query_row(
            "SELECT id, recorded_at, reason, event_count, event_kinds_json, previous_snapshot_id, snapshot_json
             FROM browser_state_snapshots
             WHERE profile_name = ?1
             ORDER BY id DESC
             LIMIT 1",
            params![profile],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<i64>>(5)?,
                    row.get::<_, String>(6)?,
                ))
            },
        )
        .optional()
        .map_err(|e| format!("Failed to query latest browser-state snapshot: {e}"))?;
    let Some((
        snapshot_id,
        recorded_at,
        reason,
        event_count,
        event_kinds_json,
        previous_snapshot_id,
        snapshot_json,
    )) = row
    else {
        return Ok(Value::Null);
    };

    let snapshot = serde_json::from_str::<Value>(&snapshot_json)
        .map_err(|e| format!("Invalid stored snapshot JSON: {e}"))?;
    let event_kinds = serde_json::from_str::<Vec<String>>(&event_kinds_json).unwrap_or_default();

    let mut stmt = conn
        .prepare(
            "SELECT logical_group_id, logical_window_id, browser_group_id, browser_window_id,
                    window_ordinal, title, color, collapsed, tab_count, tab_urls_json
             FROM browser_state_groups
             WHERE profile_name = ?1 AND snapshot_id = ?2
             ORDER BY window_ordinal ASC, browser_group_id ASC",
        )
        .map_err(|e| format!("Failed to prepare latest browser-state group query: {e}"))?;
    let rows = stmt
        .query_map(params![profile, snapshot_id], |row| {
            let tab_urls_json: String = row.get(9)?;
            let tab_urls = serde_json::from_str::<Vec<String>>(&tab_urls_json).unwrap_or_default();
            Ok(serde_json::json!({
                "logicalGroupId": row.get::<_, String>(0)?,
                "logicalWindowId": row.get::<_, String>(1)?,
                "browserGroupId": row.get::<_, i64>(2)?,
                "browserWindowId": row.get::<_, i64>(3)?,
                "windowOrdinal": row.get::<_, i64>(4)?,
                "title": row.get::<_, String>(5)?,
                "color": row.get::<_, String>(6)?,
                "collapsed": row.get::<_, Option<i64>>(7)?.map(|value| value != 0),
                "tabCount": row.get::<_, i64>(8)?,
                "tabUrls": tab_urls,
            }))
        })
        .map_err(|e| format!("Failed to query latest browser-state groups: {e}"))?;
    let mut groups = Vec::new();
    for row in rows {
        groups.push(row.map_err(|e| format!("Failed to read browser-state group row: {e}"))?);
    }

    Ok(serde_json::json!({
        "snapshotId": snapshot_id,
        "recordedAt": recorded_at,
        "reason": reason,
        "eventCount": event_count,
        "eventKinds": event_kinds,
        "previousSnapshotId": previous_snapshot_id,
        "snapshot": snapshot,
        "groups": groups,
    }))
}

pub(super) fn list_events(
    db_path: &Path,
    profile: Option<&str>,
    limit: Option<usize>,
    kind: Option<&str>,
) -> Result<Value, String> {
    let profile = profile_name(profile);
    let conn = open_db(db_path)?;
    let limit = limit.unwrap_or(DEFAULT_EVENT_LIMIT) as i64;
    let mut items = Vec::new();
    if let Some(kind) = kind {
        let mut stmt = conn
            .prepare(
                "SELECT id, recorded_at, reason, before_snapshot_id, after_snapshot_id, kind,
                        browser_window_id, browser_group_id, browser_tab_id, browser_tab_url,
                        payload_json
                 FROM browser_state_events
                 WHERE profile_name = ?1 AND kind = ?2
                 ORDER BY id DESC
                 LIMIT ?3",
            )
            .map_err(|e| format!("Failed to prepare filtered browser-state event query: {e}"))?;
        let rows = stmt
            .query_map(params![profile, kind, limit], map_event_row)
            .map_err(|e| format!("Failed to query browser-state events: {e}"))?;
        for row in rows {
            items.push(row.map_err(|e| format!("Failed to read browser-state event row: {e}"))?);
        }
    } else {
        let mut stmt = conn
            .prepare(
                "SELECT id, recorded_at, reason, before_snapshot_id, after_snapshot_id, kind,
                        browser_window_id, browser_group_id, browser_tab_id, browser_tab_url,
                        payload_json
                 FROM browser_state_events
                 WHERE profile_name = ?1
                 ORDER BY id DESC
                 LIMIT ?2",
            )
            .map_err(|e| format!("Failed to prepare browser-state event query: {e}"))?;
        let rows = stmt
            .query_map(params![profile, limit], map_event_row)
            .map_err(|e| format!("Failed to query browser-state events: {e}"))?;
        for row in rows {
            items.push(row.map_err(|e| format!("Failed to read browser-state event row: {e}"))?);
        }
    }
    Ok(Value::Array(items))
}

fn map_event_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(serde_json::json!({
        "eventId": row.get::<_, i64>(0)?,
        "recordedAt": row.get::<_, i64>(1)?,
        "reason": row.get::<_, String>(2)?,
        "beforeSnapshotId": row.get::<_, Option<i64>>(3)?,
        "afterSnapshotId": row.get::<_, i64>(4)?,
        "kind": row.get::<_, String>(5)?,
        "browserWindowId": row.get::<_, Option<i64>>(6)?,
        "browserGroupId": row.get::<_, Option<i64>>(7)?,
        "browserTabId": row.get::<_, Option<i64>>(8)?,
        "browserTabUrl": row.get::<_, Option<String>>(9)?,
        "payloadJson": row.get::<_, String>(10)?,
    }))
}

pub(super) fn list_group_history(
    db_path: &Path,
    profile: Option<&str>,
    limit: Option<usize>,
    title: Option<&str>,
    logical_group_id: Option<&str>,
) -> Result<Value, String> {
    let profile = profile_name(profile);
    let conn = open_db(db_path)?;
    let limit = limit.unwrap_or(DEFAULT_EVENT_LIMIT) as i64;
    let mut items = Vec::new();
    match (title, logical_group_id) {
        (Some(title), Some(logical_group_id)) => {
            let mut stmt = conn
                .prepare(
                    "SELECT g.logical_group_id, g.logical_window_id, g.browser_group_id, g.browser_window_id,
                            g.window_ordinal, g.title, g.color, g.collapsed, g.tab_count, g.tab_urls_json,
                            s.id, s.recorded_at, s.reason
                     FROM browser_state_groups g
                     JOIN browser_state_snapshots s ON s.id = g.snapshot_id
                     WHERE g.profile_name = ?1 AND g.title = ?2 AND g.logical_group_id = ?3
                     ORDER BY g.snapshot_id DESC
                     LIMIT ?4",
                )
                .map_err(|e| format!("Failed to prepare browser-state group history query: {e}"))?;
            let rows = stmt
                .query_map(
                    params![profile, title, logical_group_id, limit],
                    map_group_history_row,
                )
                .map_err(|e| format!("Failed to query browser-state group history: {e}"))?;
            for row in rows {
                items.push(
                    row.map_err(|e| {
                        format!("Failed to read browser-state group history row: {e}")
                    })?,
                );
            }
        }
        (Some(title), None) => {
            let mut stmt = conn
                .prepare(
                    "SELECT g.logical_group_id, g.logical_window_id, g.browser_group_id, g.browser_window_id,
                            g.window_ordinal, g.title, g.color, g.collapsed, g.tab_count, g.tab_urls_json,
                            s.id, s.recorded_at, s.reason
                     FROM browser_state_groups g
                     JOIN browser_state_snapshots s ON s.id = g.snapshot_id
                     WHERE g.profile_name = ?1 AND g.title = ?2
                     ORDER BY g.snapshot_id DESC
                     LIMIT ?3",
                )
                .map_err(|e| format!("Failed to prepare browser-state group history query: {e}"))?;
            let rows = stmt
                .query_map(params![profile, title, limit], map_group_history_row)
                .map_err(|e| format!("Failed to query browser-state group history: {e}"))?;
            for row in rows {
                items.push(
                    row.map_err(|e| {
                        format!("Failed to read browser-state group history row: {e}")
                    })?,
                );
            }
        }
        (None, Some(logical_group_id)) => {
            let mut stmt = conn
                .prepare(
                    "SELECT g.logical_group_id, g.logical_window_id, g.browser_group_id, g.browser_window_id,
                            g.window_ordinal, g.title, g.color, g.collapsed, g.tab_count, g.tab_urls_json,
                            s.id, s.recorded_at, s.reason
                     FROM browser_state_groups g
                     JOIN browser_state_snapshots s ON s.id = g.snapshot_id
                     WHERE g.profile_name = ?1 AND g.logical_group_id = ?2
                     ORDER BY g.snapshot_id DESC
                     LIMIT ?3",
                )
                .map_err(|e| format!("Failed to prepare browser-state group history query: {e}"))?;
            let rows = stmt
                .query_map(
                    params![profile, logical_group_id, limit],
                    map_group_history_row,
                )
                .map_err(|e| format!("Failed to query browser-state group history: {e}"))?;
            for row in rows {
                items.push(
                    row.map_err(|e| {
                        format!("Failed to read browser-state group history row: {e}")
                    })?,
                );
            }
        }
        (None, None) => {
            let mut stmt = conn
                .prepare(
                    "SELECT g.logical_group_id, g.logical_window_id, g.browser_group_id, g.browser_window_id,
                            g.window_ordinal, g.title, g.color, g.collapsed, g.tab_count, g.tab_urls_json,
                            s.id, s.recorded_at, s.reason
                     FROM browser_state_groups g
                     JOIN browser_state_snapshots s ON s.id = g.snapshot_id
                     WHERE g.profile_name = ?1
                     ORDER BY g.snapshot_id DESC
                     LIMIT ?2",
                )
                .map_err(|e| format!("Failed to prepare browser-state group history query: {e}"))?;
            let rows = stmt
                .query_map(params![profile, limit], map_group_history_row)
                .map_err(|e| format!("Failed to query browser-state group history: {e}"))?;
            for row in rows {
                items.push(
                    row.map_err(|e| {
                        format!("Failed to read browser-state group history row: {e}")
                    })?,
                );
            }
        }
    }
    Ok(Value::Array(items))
}

fn map_group_history_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    let tab_urls_json: String = row.get(9)?;
    let tab_urls = serde_json::from_str::<Vec<String>>(&tab_urls_json).unwrap_or_default();
    Ok(serde_json::json!({
        "logicalGroupId": row.get::<_, String>(0)?,
        "logicalWindowId": row.get::<_, String>(1)?,
        "browserGroupId": row.get::<_, i64>(2)?,
        "browserWindowId": row.get::<_, i64>(3)?,
        "windowOrdinal": row.get::<_, i64>(4)?,
        "title": row.get::<_, String>(5)?,
        "color": row.get::<_, String>(6)?,
        "collapsed": row.get::<_, Option<i64>>(7)?.map(|value| value != 0),
        "tabCount": row.get::<_, i64>(8)?,
        "tabUrls": tab_urls,
        "snapshotId": row.get::<_, i64>(10)?,
        "recordedAt": row.get::<_, i64>(11)?,
        "reason": row.get::<_, String>(12)?,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct TempDbPath(std::path::PathBuf);

    impl TempDbPath {
        fn new() -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "tabctl-browser-state-{}-{}-{}.sqlite",
                now_ms(),
                std::process::id(),
                seq
            ));
            Self(path)
        }

        fn as_path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDbPath {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn sample_snapshot(title: &str, group_urls: &[&str]) -> Value {
        serde_json::json!({
            "generatedAt": 1_700_000_000_000_i64,
            "windows": [{
                "windowId": 100,
                "focused": true,
                "state": "normal",
                "tabs": [
                    {
                        "tabId": 1,
                        "windowId": 100,
                        "index": 0,
                        "url": group_urls.first().copied().unwrap_or("https://example.com"),
                        "title": "A",
                        "active": true,
                        "pinned": false,
                        "groupId": 10,
                        "groupTitle": title,
                        "groupColor": "blue",
                        "groupCollapsed": false
                    },
                    {
                        "tabId": 2,
                        "windowId": 100,
                        "index": 1,
                        "url": group_urls.get(1).copied().unwrap_or("https://example.org"),
                        "title": "B",
                        "active": false,
                        "pinned": false,
                        "groupId": 10,
                        "groupTitle": title,
                        "groupColor": "blue",
                        "groupCollapsed": false
                    }
                ],
                "groups": [{
                    "groupId": 10,
                    "title": title,
                    "color": "blue",
                    "collapsed": false
                }]
            }]
        })
    }

    #[test]
    fn ingests_and_reads_history() {
        let db = TempDbPath::new();
        let payload = serde_json::json!({
            "reason": "startup",
            "recordedAt": 1000,
            "events": [],
            "snapshot": sample_snapshot("Work", &["https://example.com", "https://example.org"])
        });
        let ingest = ingest_sync(db.as_path(), Some("edge"), &payload).expect("ingest");
        assert_eq!(ingest["ingested"].as_bool(), Some(true));

        let history = list_history(db.as_path(), Some("edge"), Some(10)).expect("history");
        let items = history.as_array().expect("history array");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["reason"].as_str(), Some("startup"));

        let latest = latest_snapshot(db.as_path(), Some("edge")).expect("latest");
        assert_eq!(latest["reason"].as_str(), Some("startup"));
        assert_eq!(latest["groups"][0]["title"].as_str(), Some("Work"));
    }

    #[test]
    fn preserves_logical_group_identity_across_rename_with_same_tabs() {
        let db = TempDbPath::new();
        ingest_sync(
            db.as_path(),
            Some("edge"),
            &serde_json::json!({
                "reason": "startup",
                "recordedAt": 1000,
                "events": [],
                "snapshot": sample_snapshot("PR Reviews", &["https://example.com", "https://example.org"])
            }),
        )
        .expect("initial ingest");

        ingest_sync(
            db.as_path(),
            Some("edge"),
            &serde_json::json!({
                "reason": "event",
                "recordedAt": 2000,
                "events": [{"kind": "tabGroups.onUpdated", "groupId": 10}],
                "snapshot": sample_snapshot("Renamed Reviews", &["https://example.com", "https://example.org"])
            }),
        )
        .expect("renamed ingest");

        let history = list_group_history(db.as_path(), Some("edge"), Some(10), None, None)
            .expect("group history");
        let items = history.as_array().expect("history items");
        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["logicalGroupId"], items[1]["logicalGroupId"]);
    }

    #[test]
    fn open_db_allows_relative_paths() {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let seq = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::path::PathBuf::from(format!(
            "tabctl-browser-state-relative-{}-{seq}.sqlite",
            now_ms()
        ));
        let conn = open_db(&path).expect("open relative db");
        drop(conn);
        assert!(path.exists());
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn excludes_incognito_windows_from_persisted_snapshots() {
        let db = TempDbPath::new();
        ingest_sync(
            db.as_path(),
            Some("edge"),
            &serde_json::json!({
                "reason": "startup",
                "recordedAt": 1000,
                "events": [],
                "snapshot": {
                    "generatedAt": 1000,
                    "windows": [
                        {
                            "windowId": 100,
                            "focused": true,
                            "tabs": [{"tabId": 1, "windowId": 100, "incognito": false, "index": 0, "url": "https://example.com", "title": "Visible", "active": true, "pinned": false, "groupId": -1}],
                            "groups": []
                        },
                        {
                            "windowId": 200,
                            "focused": false,
                            "incognito": true,
                            "tabs": [{"tabId": 2, "windowId": 200, "incognito": true, "index": 0, "url": "https://secret.example", "title": "Secret", "active": false, "pinned": false, "groupId": -1}],
                            "groups": []
                        }
                    ]
                }
            }),
        )
        .expect("ingest");

        let latest = latest_snapshot(db.as_path(), Some("edge")).expect("latest");
        let windows = latest["snapshot"]["windows"].as_array().expect("windows");
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0]["windowId"].as_i64(), Some(100));
    }

    #[test]
    fn preserves_normal_removal_events_while_dropping_incognito_events() {
        let db = TempDbPath::new();
        ingest_sync(
            db.as_path(),
            Some("edge"),
            &serde_json::json!({
                "reason": "event",
                "recordedAt": 1000,
                "events": [
                    {"kind": "tabs.onRemoved", "tabId": 999, "windowId": 100},
                    {"kind": "tabs.onRemoved", "tabId": 555, "windowId": 200, "incognito": true}
                ],
                "snapshot": {
                    "generatedAt": 1000,
                    "windows": [
                        {
                            "windowId": 100,
                            "focused": true,
                            "tabs": [{"tabId": 1, "windowId": 100, "incognito": false, "index": 0, "url": "https://example.com", "title": "Visible", "active": true, "pinned": false, "groupId": -1}],
                            "groups": []
                        },
                        {
                            "windowId": 200,
                            "focused": false,
                            "incognito": true,
                            "tabs": [{"tabId": 2, "windowId": 200, "incognito": true, "index": 0, "url": "https://secret.example", "title": "Secret", "active": false, "pinned": false, "groupId": -1}],
                            "groups": []
                        }
                    ]
                }
            }),
        )
        .expect("ingest");

        let events = list_events(db.as_path(), Some("edge"), Some(10), None).expect("events");
        let items = events.as_array().expect("event items");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["kind"], "tabs.onRemoved");
        assert_eq!(items[0]["browserTabId"].as_i64(), Some(999));
    }

    #[test]
    fn migrates_existing_events_table_with_tab_url_column() {
        let db = TempDbPath::new();
        let conn = Connection::open(db.as_path()).expect("create old db");
        conn.execute_batch(
            r#"
            CREATE TABLE browser_state_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                profile_name TEXT NOT NULL,
                recorded_at INTEGER NOT NULL,
                reason TEXT NOT NULL,
                before_snapshot_id INTEGER,
                after_snapshot_id INTEGER NOT NULL,
                kind TEXT NOT NULL,
                browser_window_id INTEGER,
                browser_group_id INTEGER,
                browser_tab_id INTEGER,
                payload_json TEXT NOT NULL
            );
            "#,
        )
        .expect("create old events table");
        drop(conn);

        let conn = open_db(db.as_path()).expect("migrate db");
        let has_column: bool = conn
            .query_row(
                "SELECT COUNT(*) > 0
                 FROM pragma_table_info('browser_state_events')
                 WHERE name = 'browser_tab_url'",
                [],
                |row| row.get(0),
            )
            .expect("column check");
        let has_index: bool = conn
            .query_row(
                "SELECT COUNT(*) > 0
                 FROM sqlite_master
                 WHERE type = 'index' AND name = 'idx_browser_state_events_tab_url'",
                [],
                |row| row.get(0),
            )
            .expect("index check");

        assert!(has_column);
        assert!(has_index);
    }

    #[test]
    fn prunes_replaced_event_snapshots_by_normalized_url() {
        let db = TempDbPath::new();
        for idx in 0..3 {
            ingest_sync(
                db.as_path(),
                Some("edge"),
                &serde_json::json!({
                    "reason": "event",
                    "recordedAt": 1000 + idx as i64,
                    "events": [{
                        "kind": "tabs.onUpdated",
                        "tabId": 1,
                        "changeInfo": { "title": format!("tick-{idx}") }
                    }],
                    "snapshot": sample_snapshot("Work", &["https://example.com/page#frag", "https://example.org/other"])
                }),
            )
            .expect("ingest");
        }
        ingest_sync(
            db.as_path(),
            Some("edge"),
            &serde_json::json!({
                "reason": "event",
                "recordedAt": 2000,
                "events": [{
                    "kind": "tabs.onUpdated",
                    "tabId": 2,
                    "changeInfo": { "title": "other page" }
                }],
                "snapshot": sample_snapshot("Work", &["https://example.com/page#later", "https://example.org/other"])
            }),
        )
        .expect("ingest distinct URL");
        ingest_sync(
            db.as_path(),
            Some("edge"),
            &serde_json::json!({
                "reason": "event",
                "recordedAt": 3000,
                "events": [{
                    "kind": "tabs.onUpdated",
                    "tabId": 1,
                    "changeInfo": { "title": "latest page" }
                }],
                "snapshot": sample_snapshot("Work", &["https://example.com/page#latest", "https://example.org/other"])
            }),
        )
        .expect("ingest latest URL");

        let conn = open_db(db.as_path()).expect("open db");
        let snapshot_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM browser_state_snapshots WHERE profile_name = 'edge'",
                [],
                |row| row.get(0),
            )
            .expect("count snapshots");
        let window_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM browser_state_windows WHERE profile_name = 'edge'",
                [],
                |row| row.get(0),
            )
            .expect("count windows");
        let group_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM browser_state_groups WHERE profile_name = 'edge'",
                [],
                |row| row.get(0),
            )
            .expect("count groups");
        let event_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM browser_state_events WHERE profile_name = 'edge'",
                [],
                |row| row.get(0),
            )
            .expect("count events");

        assert_eq!(snapshot_count, 2);
        assert_eq!(window_count, 2);
        assert_eq!(group_count, 2);
        assert_eq!(event_count, 2);

        let events = list_events(db.as_path(), Some("edge"), Some(10), None).expect("events");
        let items = events.as_array().expect("event items");
        let urls = items
            .iter()
            .filter_map(|item| item["browserTabUrl"].as_str())
            .collect::<Vec<_>>();
        assert_eq!(urls, vec!["example.com/page", "example.org/other"]);
    }

    #[test]
    fn removes_snapshots_for_closed_normalized_url() {
        let db = TempDbPath::new();
        ingest_sync(
            db.as_path(),
            Some("edge"),
            &serde_json::json!({
                "reason": "event",
                "recordedAt": 1000,
                "events": [{
                    "kind": "tabs.onUpdated",
                    "tabId": 1,
                    "changeInfo": { "title": "open" }
                }],
                "snapshot": sample_snapshot("Work", &["https://closed.example/page#open", "https://open.example/other"])
            }),
        )
        .expect("open ingest");

        ingest_sync(
            db.as_path(),
            Some("edge"),
            &serde_json::json!({
                "reason": "event",
                "recordedAt": 2000,
                "events": [{
                    "kind": "tabs.onRemoved",
                    "tabId": 1,
                    "windowId": 100
                }],
                "snapshot": {
                    "generatedAt": 1_700_000_000_001_i64,
                    "windows": [{
                        "windowId": 100,
                        "focused": true,
                        "state": "normal",
                        "tabs": [{
                            "tabId": 2,
                            "windowId": 100,
                            "index": 0,
                            "url": "https://open.example/other",
                            "title": "Still open",
                            "active": true,
                            "pinned": false,
                            "groupId": -1
                        }],
                        "groups": []
                    }]
                }
            }),
        )
        .expect("close ingest");

        let conn = open_db(db.as_path()).expect("open db");
        let snapshot_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM browser_state_snapshots WHERE profile_name = 'edge'",
                [],
                |row| row.get(0),
            )
            .expect("count snapshots");
        let event_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM browser_state_events WHERE profile_name = 'edge'",
                [],
                |row| row.get(0),
            )
            .expect("count events");

        assert_eq!(snapshot_count, 1);
        assert_eq!(event_count, 1);

        let latest = latest_snapshot(db.as_path(), Some("edge")).expect("latest");
        let tabs = latest["snapshot"]["windows"][0]["tabs"]
            .as_array()
            .expect("latest tabs");
        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs[0]["url"].as_str(), Some("https://open.example/other"));
    }

    #[test]
    fn removes_snapshots_for_replaced_tab_url() {
        let db = TempDbPath::new();
        ingest_sync(
            db.as_path(),
            Some("edge"),
            &serde_json::json!({
                "reason": "event",
                "recordedAt": 1000,
                "events": [{
                    "kind": "tabs.onUpdated",
                    "tabId": 1,
                    "changeInfo": { "title": "old page" }
                }],
                "snapshot": sample_snapshot("Work", &["https://old.example/page#old", "https://open.example/other"])
            }),
        )
        .expect("old URL ingest");

        ingest_sync(
            db.as_path(),
            Some("edge"),
            &serde_json::json!({
                "reason": "event",
                "recordedAt": 2000,
                "events": [{
                    "kind": "tabs.onUpdated",
                    "tabId": 1,
                    "changeInfo": { "url": "https://new.example/page#new" }
                }],
                "snapshot": sample_snapshot("Work", &["https://new.example/page#new", "https://open.example/other"])
            }),
        )
        .expect("new URL ingest");

        let conn = open_db(db.as_path()).expect("open db");
        let snapshot_count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM browser_state_snapshots WHERE profile_name = 'edge'",
                [],
                |row| row.get(0),
            )
            .expect("count snapshots");
        let old_url_events: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM browser_state_events WHERE profile_name = 'edge' AND browser_tab_url = 'old.example/page'",
                [],
                |row| row.get(0),
            )
            .expect("count old URL events");

        assert_eq!(snapshot_count, 1);
        assert_eq!(old_url_events, 0);

        let latest = latest_snapshot(db.as_path(), Some("edge")).expect("latest");
        assert_eq!(
            latest["snapshot"]["windows"][0]["tabs"][0]["url"].as_str(),
            Some("https://new.example/page#new")
        );
    }
}
