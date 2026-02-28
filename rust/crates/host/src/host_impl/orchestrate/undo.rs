use serde_json::{Map, Value};
use std::collections::HashMap;

use super::OrchStep;

/// Orchestration for the `undo` command.
///
/// Reads the undo record and sequences restoration primitives based on
/// the original action type. The general pattern is:
///
/// p:snapshot → [p:window-create × missing] →
///   close: p:tab-create × N |
///   others: p:tab-move × batches →
/// p:tab-group + p:group-update × groups → Complete
///
/// For `group-update` undo, a single p:group-update suffices.
#[derive(Debug)]
pub(crate) struct UndoOrchestration {
    params: Value,
    phase: Phase,
    state: UndoState,
}

#[derive(Debug)]
enum Phase {
    Snapshot,
    CreateWindow,
    CreateTab,
    MoveBatch,
    GroupTabs,
    UpdateGroup,
    GroupUpdateDone,
}

#[derive(Debug, Default)]
struct UndoState {
    undo_action: String,
    entries: Vec<UndoEntry>,

    windows_to_create: Vec<i64>,
    window_create_idx: usize,
    window_map: HashMap<i64, i64>,

    creates: Vec<usize>,
    create_idx: usize,
    move_batches: Vec<MoveBatch>,
    move_batch_idx: usize,
    created_tabs: HashMap<usize, i64>,

    groups: Vec<GroupRestore>,
    group_idx: usize,
    current_group_id: Option<i64>,

    skipped: Vec<Value>,
    restored_count: usize,
}

#[derive(Debug)]
struct UndoEntry {
    tab_id: Option<i64>,
    url: Option<String>,
    pinned: bool,
    #[allow(dead_code)]
    active: bool,
    window_id: i64,
    #[allow(dead_code)]
    index: Option<i64>,
    group_id: i64,
    group_title: Option<String>,
    group_color: Option<String>,
    group_collapsed: Option<bool>,
}

#[derive(Debug)]
struct MoveBatch {
    window_id: i64,
    tab_ids: Vec<i64>,
}

#[derive(Debug)]
struct GroupRestore {
    window_id: i64,
    title: Option<String>,
    color: Option<String>,
    collapsed: Option<bool>,
    tab_ids: Vec<i64>,
}

impl UndoOrchestration {
    pub(crate) fn new(params: &Value) -> Self {
        Self {
            params: params.clone(),
            phase: Phase::Snapshot,
            state: UndoState::default(),
        }
    }
}

impl super::Orchestration for UndoOrchestration {
    fn start(&mut self) -> OrchStep {
        let record = match self.params.get("record").and_then(Value::as_object) {
            Some(r) => r.clone(),
            None => {
                return OrchStep::Error {
                    message: "Undo record missing".to_string(),
                    hint: None,
                }
            }
        };

        let undo = match record.get("undo").and_then(Value::as_object) {
            Some(u) => u.clone(),
            None => {
                return OrchStep::Error {
                    message: "Undo payload missing".to_string(),
                    hint: None,
                }
            }
        };

        let undo_action = undo
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();

        if undo_action.is_empty() {
            return OrchStep::Error {
                message: "Unknown undo action".to_string(),
                hint: None,
            };
        }

        // Simple case: group-update is a single p:group-update
        if undo_action == "group-update" {
            return self.start_group_update(&undo);
        }

        // Validate action is known
        match undo_action.as_str() {
            "close" | "archive" | "merge-window" | "group-ungroup" | "group-assign"
            | "group-gather" | "move-tab" | "move-group" => {}
            _ => {
                return OrchStep::Error {
                    message: format!("Unknown undo action: {undo_action}"),
                    hint: None,
                }
            }
        }

        self.state.undo_action = undo_action.clone();
        self.state.entries = normalize_entries(&undo_action, &Value::Object(undo));

        if self.state.entries.is_empty() {
            return OrchStep::Complete {
                response: serde_json::json!({
                    "summary": { "restoredTabs": 0, "skippedTabs": 0 },
                    "skipped": [],
                }),
                undo: None,
            };
        }

        self.phase = Phase::Snapshot;
        OrchStep::SendPrimitive {
            action: "p:snapshot".to_string(),
            params: Value::Object(Map::new()),
        }
    }

    fn step(&mut self, response: Value) -> OrchStep {
        match self.phase {
            Phase::Snapshot => self.handle_snapshot(response),
            Phase::CreateWindow => self.handle_window_created(response),
            Phase::CreateTab => self.handle_tab_created(response),
            Phase::MoveBatch => self.handle_batch_moved(),
            Phase::GroupTabs => self.handle_grouped(response),
            Phase::UpdateGroup => self.handle_group_updated(),
            Phase::GroupUpdateDone => self.complete_group_update(),
        }
    }
}

impl UndoOrchestration {
    fn start_group_update(&mut self, undo: &Map<String, Value>) -> OrchStep {
        let group_id = match undo.get("groupId").and_then(Value::as_i64) {
            Some(gid) => gid,
            None => {
                return OrchStep::Error {
                    message: "Missing groupId in undo record".to_string(),
                    hint: None,
                }
            }
        };

        let previous = undo.get("previous").and_then(Value::as_object);
        let mut update = Map::new();
        if let Some(prev) = previous {
            if let Some(title) = prev.get("title").and_then(Value::as_str) {
                update.insert("title".to_string(), Value::String(title.to_string()));
            }
            if let Some(color) = prev.get("color").and_then(Value::as_str) {
                if !color.is_empty() {
                    update.insert("color".to_string(), Value::String(color.to_string()));
                }
            }
            if let Some(collapsed) = prev.get("collapsed").and_then(Value::as_bool) {
                update.insert("collapsed".to_string(), Value::Bool(collapsed));
            }
        }

        if update.is_empty() {
            return OrchStep::Error {
                message: "No previous values in undo record".to_string(),
                hint: None,
            };
        }

        self.phase = Phase::GroupUpdateDone;
        update.insert("groupId".to_string(), serde_json::json!(group_id));
        OrchStep::SendPrimitive {
            action: "p:group-update".to_string(),
            params: Value::Object(update),
        }
    }

    fn handle_snapshot(&mut self, snapshot: Value) -> OrchStep {
        let mut existing: Vec<i64> = Vec::new();
        if let Some(windows) = snapshot.get("windows").and_then(Value::as_array) {
            for win in windows {
                if let Some(wid) = win.get("windowId").and_then(Value::as_i64) {
                    existing.push(wid);
                }
            }
        }

        // Find unique target windows and check existence
        let mut seen: Vec<i64> = Vec::new();
        for entry in &self.state.entries {
            if !seen.contains(&entry.window_id) {
                seen.push(entry.window_id);
            }
        }

        for &wid in &seen {
            if existing.contains(&wid) {
                self.state.window_map.insert(wid, wid);
            } else {
                self.state.windows_to_create.push(wid);
            }
        }

        if !self.state.windows_to_create.is_empty() {
            self.phase = Phase::CreateWindow;
            return OrchStep::SendPrimitive {
                action: "p:window-create".to_string(),
                params: serde_json::json!({"focused": false}),
            };
        }

        self.build_plan_and_start()
    }

    fn handle_window_created(&mut self, response: Value) -> OrchStep {
        let window_id = response
            .get("id")
            .or_else(|| response.get("windowId"))
            .and_then(Value::as_i64);

        if let Some(new_wid) = window_id {
            let orig_wid = self.state.windows_to_create[self.state.window_create_idx];
            self.state.window_map.insert(orig_wid, new_wid);
        }

        self.state.window_create_idx += 1;

        if self.state.window_create_idx < self.state.windows_to_create.len() {
            return OrchStep::SendPrimitive {
                action: "p:window-create".to_string(),
                params: serde_json::json!({"focused": false}),
            };
        }

        self.build_plan_and_start()
    }

    fn build_plan_and_start(&mut self) -> OrchStep {
        let mut creates: Vec<usize> = Vec::new();
        let mut move_batches: Vec<MoveBatch> = Vec::new();
        let mut skipped: Vec<Value> = Vec::new();

        for (idx, entry) in self.state.entries.iter().enumerate() {
            let actual_wid = self
                .state
                .window_map
                .get(&entry.window_id)
                .copied()
                .unwrap_or(entry.window_id);

            if let Some(tab_id) = entry.tab_id {
                if let Some(batch) = move_batches.iter_mut().find(|b| b.window_id == actual_wid) {
                    batch.tab_ids.push(tab_id);
                } else {
                    move_batches.push(MoveBatch {
                        window_id: actual_wid,
                        tab_ids: vec![tab_id],
                    });
                }
            } else if entry.url.is_some() {
                creates.push(idx);
            } else {
                skipped.push(serde_json::json!({"reason": "missing_tab_and_url"}));
            }
        }

        self.state.creates = creates;
        self.state.move_batches = move_batches;
        self.state.skipped.extend(skipped);

        self.next_create_or_move()
    }

    fn next_create_or_move(&mut self) -> OrchStep {
        // Creates first (close/archive without tabId)
        if self.state.create_idx < self.state.creates.len() {
            let entry_idx = self.state.creates[self.state.create_idx];
            let entry = &self.state.entries[entry_idx];
            let actual_wid = self
                .state
                .window_map
                .get(&entry.window_id)
                .copied()
                .unwrap_or(entry.window_id);
            let url = entry.url.clone().unwrap_or_default();

            self.phase = Phase::CreateTab;
            return OrchStep::SendPrimitive {
                action: "p:tab-create".to_string(),
                params: serde_json::json!({
                    "windowId": actual_wid,
                    "url": url,
                    "active": false,
                    "pinned": entry.pinned,
                }),
            };
        }

        // Then move batches (entries with tabId)
        if self.state.move_batch_idx < self.state.move_batches.len() {
            let batch = &self.state.move_batches[self.state.move_batch_idx];

            self.phase = Phase::MoveBatch;
            return OrchStep::SendPrimitive {
                action: "p:tab-move".to_string(),
                params: serde_json::json!({
                    "tabIds": batch.tab_ids,
                    "windowId": batch.window_id,
                    "index": -1,
                }),
            };
        }

        self.start_grouping()
    }

    fn handle_tab_created(&mut self, response: Value) -> OrchStep {
        let entry_idx = self.state.creates[self.state.create_idx];

        if let Some(tab_id) = response.get("id").and_then(Value::as_i64) {
            self.state.created_tabs.insert(entry_idx, tab_id);
            self.state.restored_count += 1;
        } else {
            self.state.skipped.push(serde_json::json!({
                "entry": entry_idx,
                "reason": "create_failed",
            }));
        }

        self.state.create_idx += 1;
        self.next_create_or_move()
    }

    fn handle_batch_moved(&mut self) -> OrchStep {
        let batch = &self.state.move_batches[self.state.move_batch_idx];
        self.state.restored_count += batch.tab_ids.len();
        self.state.move_batch_idx += 1;
        self.next_create_or_move()
    }

    fn start_grouping(&mut self) -> OrchStep {
        let mut groups: Vec<GroupRestore> = Vec::new();
        let mut key_index: HashMap<String, usize> = HashMap::new();

        for (idx, entry) in self.state.entries.iter().enumerate() {
            if entry.group_id == -1 && entry.group_title.is_none() {
                continue;
            }

            let actual_wid = self
                .state
                .window_map
                .get(&entry.window_id)
                .copied()
                .unwrap_or(entry.window_id);

            // Resolve actual tab ID
            let tab_id = if let Some(tid) = entry.tab_id {
                tid
            } else if let Some(&tid) = self.state.created_tabs.get(&idx) {
                tid
            } else {
                continue;
            };

            let key = if entry.group_id != -1 {
                format!("{}:id:{}", actual_wid, entry.group_id)
            } else {
                match entry.group_title {
                    Some(ref t) => format!("{}:title:{}", actual_wid, t),
                    None => continue,
                }
            };

            if let Some(&pos) = key_index.get(&key) {
                groups[pos].tab_ids.push(tab_id);
            } else {
                let pos = groups.len();
                key_index.insert(key, pos);
                groups.push(GroupRestore {
                    window_id: actual_wid,
                    title: entry.group_title.clone(),
                    color: entry.group_color.clone(),
                    collapsed: entry.group_collapsed,
                    tab_ids: vec![tab_id],
                });
            }
        }

        self.state.groups = groups;
        self.next_group()
    }

    fn next_group(&mut self) -> OrchStep {
        if self.state.group_idx >= self.state.groups.len() {
            return self.complete();
        }

        let group = &self.state.groups[self.state.group_idx];

        self.phase = Phase::GroupTabs;
        OrchStep::SendPrimitive {
            action: "p:tab-group".to_string(),
            params: serde_json::json!({
                "tabIds": group.tab_ids,
                "createProperties": {"windowId": group.window_id},
            }),
        }
    }

    fn handle_grouped(&mut self, response: Value) -> OrchStep {
        let group_id = response
            .get("groupId")
            .and_then(Value::as_i64)
            .or_else(|| response.as_i64());

        self.state.current_group_id = group_id;

        let group = &self.state.groups[self.state.group_idx];
        let mut update = Map::new();
        if let Some(ref title) = group.title {
            update.insert("title".to_string(), Value::String(title.clone()));
        }
        if let Some(ref color) = group.color {
            if !color.is_empty() {
                update.insert("color".to_string(), Value::String(color.clone()));
            }
        }
        if let Some(collapsed) = group.collapsed {
            update.insert("collapsed".to_string(), Value::Bool(collapsed));
        }

        if !update.is_empty() {
            if let Some(gid) = group_id {
                self.phase = Phase::UpdateGroup;
                update.insert("groupId".to_string(), serde_json::json!(gid));
                return OrchStep::SendPrimitive {
                    action: "p:group-update".to_string(),
                    params: Value::Object(update),
                };
            }
        }

        self.advance_group()
    }

    fn handle_group_updated(&mut self) -> OrchStep {
        self.advance_group()
    }

    fn advance_group(&mut self) -> OrchStep {
        self.state.group_idx += 1;
        self.next_group()
    }

    fn complete_group_update(&self) -> OrchStep {
        OrchStep::Complete {
            response: serde_json::json!({
                "summary": { "restoredGroups": 1 },
            }),
            undo: None,
        }
    }

    fn complete(&self) -> OrchStep {
        OrchStep::Complete {
            response: serde_json::json!({
                "summary": {
                    "restoredTabs": self.state.restored_count,
                    "skippedTabs": self.state.skipped.len(),
                },
                "skipped": self.state.skipped,
            }),
            undo: None,
        }
    }
}

// ---------------------------------------------------------------------------
// Entry normalization helpers
// ---------------------------------------------------------------------------

/// Normalize undo entries from various record shapes into a uniform format.
fn normalize_entries(action: &str, undo: &Value) -> Vec<UndoEntry> {
    match action {
        "move-tab" => parse_move_tab_entry(undo).into_iter().collect(),
        "close" | "archive" | "merge-window" => undo
            .get("tabs")
            .and_then(Value::as_array)
            .map(|tabs| tabs.iter().filter_map(parse_from_entry).collect())
            .unwrap_or_default(),
        "group-ungroup" | "group-assign" | "group-gather" | "move-group" => undo
            .get("tabs")
            .and_then(Value::as_array)
            .map(|tabs| tabs.iter().filter_map(parse_flat_entry).collect())
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

/// Parse entry from `{"url": ..., "from": {"windowId": ..., ...}}` format
/// (close, archive, merge-window).
fn parse_from_entry(entry: &Value) -> Option<UndoEntry> {
    let from = entry.get("from")?.as_object()?;
    Some(UndoEntry {
        tab_id: entry.get("tabId").and_then(Value::as_i64),
        url: entry.get("url").and_then(Value::as_str).map(String::from),
        pinned: entry
            .get("pinned")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        active: entry
            .get("active")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        window_id: from.get("windowId").and_then(Value::as_i64)?,
        index: from.get("index").and_then(Value::as_i64),
        group_id: from.get("groupId").and_then(Value::as_i64).unwrap_or(-1),
        group_title: from
            .get("groupTitle")
            .and_then(Value::as_str)
            .map(String::from),
        group_color: from
            .get("groupColor")
            .and_then(Value::as_str)
            .map(String::from),
        group_collapsed: from.get("groupCollapsed").and_then(Value::as_bool),
    })
}

/// Parse entry from flat `{"tabId": ..., "windowId": ..., ...}` format
/// (group-ungroup, group-assign, group-gather, move-group).
fn parse_flat_entry(entry: &Value) -> Option<UndoEntry> {
    Some(UndoEntry {
        tab_id: entry.get("tabId").and_then(Value::as_i64),
        url: entry.get("url").and_then(Value::as_str).map(String::from),
        pinned: entry
            .get("pinned")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        active: entry
            .get("active")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        window_id: entry.get("windowId").and_then(Value::as_i64)?,
        index: entry.get("index").and_then(Value::as_i64),
        group_id: entry.get("groupId").and_then(Value::as_i64).unwrap_or(-1),
        group_title: entry
            .get("groupTitle")
            .and_then(Value::as_str)
            .map(String::from),
        group_color: entry
            .get("groupColor")
            .and_then(Value::as_str)
            .map(String::from),
        group_collapsed: entry.get("groupCollapsed").and_then(Value::as_bool),
    })
}

/// Parse move-tab undo: single `{"tabId": ..., "from": {...}}`.
fn parse_move_tab_entry(undo: &Value) -> Option<UndoEntry> {
    let from = undo.get("from")?.as_object()?;
    Some(UndoEntry {
        tab_id: undo.get("tabId").and_then(Value::as_i64),
        url: None,
        pinned: false,
        active: false,
        window_id: from.get("windowId").and_then(Value::as_i64)?,
        index: from.get("index").and_then(Value::as_i64),
        group_id: from.get("groupId").and_then(Value::as_i64).unwrap_or(-1),
        group_title: from
            .get("groupTitle")
            .and_then(Value::as_str)
            .map(String::from),
        group_color: from
            .get("groupColor")
            .and_then(Value::as_str)
            .map(String::from),
        group_collapsed: from.get("groupCollapsed").and_then(Value::as_bool),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host_impl::orchestrate::Orchestration;

    fn make_params(undo_record: Value) -> Value {
        serde_json::json!({"record": undo_record})
    }

    fn snapshot_with_window(window_id: i64) -> Value {
        serde_json::json!({
            "windows": [{"windowId": window_id, "focused": true, "tabs": [], "groups": []}]
        })
    }

    #[test]
    fn undo_close_creates_tabs_and_groups() {
        let record = serde_json::json!({
            "txid": "tx-1", "action": "close",
            "undo": {
                "action": "close",
                "tabs": [
                    {"url": "https://a.com", "title": "A", "pinned": false, "active": false,
                     "from": {"windowId": 100, "index": 0, "groupId": 10, "groupTitle": "Dev", "groupColor": "blue", "groupCollapsed": false}},
                    {"url": "https://b.com", "title": "B", "pinned": true, "active": false,
                     "from": {"windowId": 100, "index": 1, "groupId": -1}}
                ]
            }
        });
        let mut orch = UndoOrchestration::new(&make_params(record));

        // start → p:snapshot
        let step = orch.start();
        assert!(matches!(&step, OrchStep::SendPrimitive { action, .. } if action == "p:snapshot"));

        // Snapshot (window 100 exists) → p:tab-create first tab
        let step = orch.step(snapshot_with_window(100));
        let OrchStep::SendPrimitive { action, params } = &step else {
            panic!("expected tab-create, got {step:?}");
        };
        assert_eq!(action, "p:tab-create");
        assert_eq!(params["url"], "https://a.com");
        assert_eq!(params["windowId"], 100);

        // Tab created → p:tab-create second tab
        let step = orch.step(serde_json::json!({"id": 51, "windowId": 100}));
        let OrchStep::SendPrimitive { action, params } = &step else {
            panic!("expected tab-create, got {step:?}");
        };
        assert_eq!(action, "p:tab-create");
        assert_eq!(params["url"], "https://b.com");
        assert_eq!(params["pinned"], true);

        // Second tab created → p:tab-group (only tab in group "Dev")
        let step = orch.step(serde_json::json!({"id": 52, "windowId": 100}));
        let OrchStep::SendPrimitive { action, params } = &step else {
            panic!("expected tab-group, got {step:?}");
        };
        assert_eq!(action, "p:tab-group");
        assert_eq!(params["tabIds"], serde_json::json!([51]));

        // Grouped → p:group-update
        let step = orch.step(serde_json::json!({"groupId": 60}));
        let OrchStep::SendPrimitive { action, params } = &step else {
            panic!("expected group-update, got {step:?}");
        };
        assert_eq!(action, "p:group-update");
        assert_eq!(params["title"], "Dev");
        assert_eq!(params["color"], "blue");

        // Updated → Complete
        let step = orch.step(serde_json::json!({}));
        let OrchStep::Complete { response, .. } = step else {
            panic!("expected Complete");
        };
        assert_eq!(response["summary"]["restoredTabs"], 2);
    }

    #[test]
    fn undo_group_update_simple() {
        let record = serde_json::json!({
            "txid": "tx-2", "action": "group-update",
            "undo": {
                "action": "group-update",
                "groupId": 10,
                "windowId": 100,
                "previous": {"title": "OldTitle", "color": "red", "collapsed": true}
            }
        });
        let mut orch = UndoOrchestration::new(&make_params(record));

        // start → p:group-update directly (no snapshot needed)
        let step = orch.start();
        let OrchStep::SendPrimitive { action, params } = &step else {
            panic!("expected group-update, got {step:?}");
        };
        assert_eq!(action, "p:group-update");
        assert_eq!(params["groupId"], 10);
        assert_eq!(params["title"], "OldTitle");
        assert_eq!(params["color"], "red");
        assert_eq!(params["collapsed"], true);

        // Updated → Complete
        let step = orch.step(serde_json::json!({}));
        let OrchStep::Complete { response, .. } = step else {
            panic!("expected Complete");
        };
        assert_eq!(response["summary"]["restoredGroups"], 1);
    }

    #[test]
    fn undo_move_tab_single() {
        let record = serde_json::json!({
            "txid": "tx-3", "action": "move-tab",
            "undo": {
                "action": "move-tab",
                "tabId": 5,
                "from": {"windowId": 100, "index": 2, "groupId": 10, "groupTitle": "Dev", "groupColor": "blue", "groupCollapsed": false},
                "to": {"windowId": 200, "index": 0}
            }
        });
        let mut orch = UndoOrchestration::new(&make_params(record));

        let step = orch.start();
        assert!(matches!(&step, OrchStep::SendPrimitive { action, .. } if action == "p:snapshot"));

        // Snapshot → p:tab-move back to window 100
        let step = orch.step(snapshot_with_window(100));
        let OrchStep::SendPrimitive { action, params } = &step else {
            panic!("expected tab-move, got {step:?}");
        };
        assert_eq!(action, "p:tab-move");
        assert_eq!(params["tabIds"], serde_json::json!([5]));
        assert_eq!(params["windowId"], 100);

        // Moved → p:tab-group (restore group "Dev")
        let step = orch.step(serde_json::json!({}));
        let OrchStep::SendPrimitive { action, params } = &step else {
            panic!("expected tab-group, got {step:?}");
        };
        assert_eq!(action, "p:tab-group");
        assert_eq!(params["tabIds"], serde_json::json!([5]));

        // Grouped → p:group-update
        let step = orch.step(serde_json::json!({"groupId": 60}));
        let OrchStep::SendPrimitive { action, params } = &step else {
            panic!("expected group-update, got {step:?}");
        };
        assert_eq!(action, "p:group-update");
        assert_eq!(params["title"], "Dev");

        // Updated → Complete
        let step = orch.step(serde_json::json!({}));
        let OrchStep::Complete { response, .. } = step else {
            panic!("expected Complete");
        };
        assert_eq!(response["summary"]["restoredTabs"], 1);
    }

    #[test]
    fn undo_group_ungroup_regroups() {
        let record = serde_json::json!({
            "txid": "tx-4", "action": "group-ungroup",
            "undo": {
                "action": "group-ungroup",
                "groupId": 10, "windowId": 100,
                "groupTitle": "Dev", "groupColor": "blue", "groupCollapsed": false,
                "tabs": [
                    {"tabId": 1, "windowId": 100, "index": 0, "groupId": 10, "groupTitle": "Dev", "groupColor": "blue", "groupCollapsed": false},
                    {"tabId": 2, "windowId": 100, "index": 1, "groupId": 10, "groupTitle": "Dev", "groupColor": "blue", "groupCollapsed": false}
                ]
            }
        });
        let mut orch = UndoOrchestration::new(&make_params(record));

        let step = orch.start();
        assert!(matches!(&step, OrchStep::SendPrimitive { action, .. } if action == "p:snapshot"));

        // Snapshot → p:tab-move (batch to window 100)
        let step = orch.step(snapshot_with_window(100));
        let OrchStep::SendPrimitive { action, params } = &step else {
            panic!("expected tab-move, got {step:?}");
        };
        assert_eq!(action, "p:tab-move");
        assert_eq!(params["tabIds"], serde_json::json!([1, 2]));

        // Moved → p:tab-group
        let step = orch.step(serde_json::json!({}));
        let OrchStep::SendPrimitive { action, params } = &step else {
            panic!("expected tab-group, got {step:?}");
        };
        assert_eq!(action, "p:tab-group");
        assert_eq!(params["tabIds"], serde_json::json!([1, 2]));

        // Grouped → p:group-update
        let step = orch.step(serde_json::json!({"groupId": 60}));
        let OrchStep::SendPrimitive { action, .. } = &step else {
            panic!("expected group-update, got {step:?}");
        };
        assert_eq!(action, "p:group-update");

        // Updated → Complete
        let step = orch.step(serde_json::json!({}));
        let OrchStep::Complete { response, .. } = step else {
            panic!("expected Complete");
        };
        assert_eq!(response["summary"]["restoredTabs"], 2);
    }

    #[test]
    fn undo_missing_window_creates_it() {
        let record = serde_json::json!({
            "txid": "tx-5", "action": "close",
            "undo": {
                "action": "close",
                "tabs": [
                    {"url": "https://a.com", "from": {"windowId": 999, "index": 0, "groupId": -1}}
                ]
            }
        });
        let mut orch = UndoOrchestration::new(&make_params(record));

        let step = orch.start();
        assert!(matches!(&step, OrchStep::SendPrimitive { action, .. } if action == "p:snapshot"));

        // Snapshot with no window 999 → p:window-create
        let step = orch.step(snapshot_with_window(100));
        let OrchStep::SendPrimitive { action, .. } = &step else {
            panic!("expected window-create, got {step:?}");
        };
        assert_eq!(action, "p:window-create");

        // Window created (new id 300) → p:tab-create in window 300
        let step = orch.step(serde_json::json!({"id": 300}));
        let OrchStep::SendPrimitive { action, params } = &step else {
            panic!("expected tab-create, got {step:?}");
        };
        assert_eq!(action, "p:tab-create");
        assert_eq!(params["windowId"], 300);

        // Tab created → Complete (no groups to restore)
        let step = orch.step(serde_json::json!({"id": 70}));
        let OrchStep::Complete { response, .. } = step else {
            panic!("expected Complete");
        };
        assert_eq!(response["summary"]["restoredTabs"], 1);
    }

    #[test]
    fn undo_empty_tabs_completes_immediately() {
        let record = serde_json::json!({
            "txid": "tx-6", "action": "close",
            "undo": {"action": "close", "tabs": []}
        });
        let mut orch = UndoOrchestration::new(&make_params(record));

        let step = orch.start();
        let OrchStep::Complete { response, .. } = step else {
            panic!("expected Complete for empty tabs");
        };
        assert_eq!(response["summary"]["restoredTabs"], 0);
    }

    #[test]
    fn undo_missing_record_errors() {
        let params = serde_json::json!({});
        let mut orch = UndoOrchestration::new(&params);

        let step = orch.start();
        assert!(matches!(&step, OrchStep::Error { message, .. } if message.contains("missing")));
    }

    #[test]
    fn undo_unknown_action_errors() {
        let record = serde_json::json!({
            "txid": "tx-7", "action": "unknown",
            "undo": {"action": "unknown_action"}
        });
        let mut orch = UndoOrchestration::new(&make_params(record));

        let step = orch.start();
        assert!(
            matches!(&step, OrchStep::Error { message, .. } if message.contains("Unknown undo action"))
        );
    }

    #[test]
    fn undo_merge_window_moves_tabs_back() {
        let record = serde_json::json!({
            "txid": "tx-8", "action": "merge-window",
            "undo": {
                "action": "merge-window",
                "fromWindowId": 100, "toWindowId": 200, "closedSource": false,
                "tabs": [
                    {"tabId": 1, "url": "https://a.com", "title": "A", "pinned": false, "active": true,
                     "from": {"windowId": 100, "index": 0, "groupId": 10, "groupTitle": "Dev", "groupColor": "blue", "groupCollapsed": false}},
                    {"tabId": 2, "url": "https://b.com", "title": "B", "pinned": false, "active": false,
                     "from": {"windowId": 100, "index": 1, "groupId": -1}}
                ]
            }
        });
        let mut orch = UndoOrchestration::new(&make_params(record));

        let step = orch.start();
        assert!(matches!(&step, OrchStep::SendPrimitive { action, .. } if action == "p:snapshot"));

        // Snapshot → tab-move batch to window 100
        let step = orch.step(snapshot_with_window(100));
        let OrchStep::SendPrimitive { action, params } = &step else {
            panic!("expected tab-move, got {step:?}");
        };
        assert_eq!(action, "p:tab-move");
        assert_eq!(params["tabIds"], serde_json::json!([1, 2]));
        assert_eq!(params["windowId"], 100);

        // Moved → p:tab-group (only tab 1 has group)
        let step = orch.step(serde_json::json!({}));
        let OrchStep::SendPrimitive { action, params } = &step else {
            panic!("expected tab-group, got {step:?}");
        };
        assert_eq!(action, "p:tab-group");
        assert_eq!(params["tabIds"], serde_json::json!([1]));

        // Grouped → p:group-update
        let step = orch.step(serde_json::json!({"groupId": 60}));
        assert!(
            matches!(&step, OrchStep::SendPrimitive { action, .. } if action == "p:group-update")
        );

        // Updated → Complete
        let step = orch.step(serde_json::json!({}));
        let OrchStep::Complete { response, .. } = step else {
            panic!("expected Complete");
        };
        assert_eq!(response["summary"]["restoredTabs"], 2);
    }
}
