use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};

use super::entries::{normalize, Entry};
use crate::host_impl::orchestrate::scope::select_tabs_by_scope;
use crate::host_impl::orchestrate::{OrchStep, Orchestration};

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

#[derive(Debug)]
pub(crate) struct UndoOrchestration {
    undo: Value,
    phase: Phase,
    entries: Vec<Entry>,
    entry_idx: usize,
    tab_ids: HashMap<usize, i64>,
    window_map: HashMap<i64, i64>,
    missing_windows: Vec<i64>,
    window_idx: usize,
    existing_groups: HashSet<(i64, i64)>,
    groups: Vec<Group>,
    group_idx: usize,
    ungrouped: Vec<i64>,
    cleanup: Vec<i64>,
    positions: HashMap<i64, (i64, i64)>,
    pins: HashMap<i64, bool>,
    active: Vec<i64>,
    skipped: Vec<Value>,
    restored: usize,
}

#[derive(Debug)]
enum Phase {
    Snapshot,
    Window,
    Create,
    Move,
    PrePin,
    GroupSnapshot,
    Ungroup,
    Group,
    UpdateGroup,
    Activate,
    Cleanup,
    LegacyGroup,
}

#[derive(Debug)]
struct Group {
    window_id: i64,
    original_id: i64,
    title: Option<String>,
    color: Option<String>,
    collapsed: Option<bool>,
    tab_ids: Vec<i64>,
}

impl UndoOrchestration {
    pub(crate) fn new(params: &Value) -> Self {
        Self {
            undo: params["record"]["undo"].clone(),
            phase: Phase::Snapshot,
            entries: Vec::new(),
            entry_idx: 0,
            tab_ids: HashMap::new(),
            window_map: HashMap::new(),
            missing_windows: Vec::new(),
            window_idx: 0,
            existing_groups: HashSet::new(),
            groups: Vec::new(),
            group_idx: 0,
            ungrouped: Vec::new(),
            cleanup: Vec::new(),
            active: Vec::new(),
            positions: HashMap::new(),
            pins: HashMap::new(),
            skipped: Vec::new(),
            restored: 0,
        }
    }

    fn send(action: &str, params: Value) -> OrchStep {
        OrchStep::SendPrimitive {
            action: action.into(),
            params,
        }
    }

    fn error(message: impl Into<String>) -> OrchStep {
        OrchStep::Error {
            message: message.into(),
            hint: None,
        }
    }

    fn snapshot(&mut self, snapshot: Value) -> OrchStep {
        let tabs = select_tabs_by_scope(&snapshot, &json!({"all":true})).tabs;
        let existing: HashMap<i64, i64> =
            tabs.iter().map(|tab| (tab.tab_id, tab.window_id)).collect();
        self.positions = tabs
            .iter()
            .map(|tab| (tab.tab_id, (tab.window_id, tab.index.unwrap_or(-1))))
            .collect();
        self.pins = tabs
            .iter()
            .filter_map(|tab| tab.pinned.map(|pinned| (tab.tab_id, pinned)))
            .collect();
        let mut entries = Vec::new();
        for mut entry in std::mem::take(&mut self.entries) {
            if let Some(id) = entry.tab_id {
                if !existing.contains_key(&id) {
                    if entry.create_if_missing && entry.url.is_some() {
                        entry.tab_id = None;
                    } else {
                        self.skipped.push(json!({"tabId":id,"reason":"not_found"}));
                        continue;
                    }
                }
            } else if entry.url.is_none() {
                self.skipped.push(json!({"reason":"missing_tab_and_url"}));
                continue;
            }
            entries.push(entry);
        }
        self.entries = entries;
        if let Some(windows) = snapshot["windows"].as_array() {
            for window in windows {
                if let Some(id) = window["windowId"].as_i64() {
                    self.window_map.insert(id, id);
                    for group in window["groups"].as_array().into_iter().flatten() {
                        if let Some(group_id) = group["groupId"].as_i64() {
                            self.existing_groups.insert((id, group_id));
                        }
                    }
                }
            }
        }
        for entry in &self.entries {
            if !self.window_map.contains_key(&entry.window_id)
                && !self.missing_windows.contains(&entry.window_id)
            {
                self.missing_windows.push(entry.window_id);
            }
        }
        self.cleanup = self.undo["cleanupTabs"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_i64)
            .filter(|id| existing.contains_key(id))
            .collect();
        self.next_window()
    }

    fn next_window(&mut self) -> OrchStep {
        if self.window_idx >= self.missing_windows.len() {
            return self.next_entry();
        }
        let window_id = self.missing_windows[self.window_idx];
        let entry = self
            .entries
            .iter()
            .find(|entry| entry.window_id == window_id)
            .unwrap();
        self.phase = Phase::Window;
        // Use a tab being restored as the initial tab. No unrelated seed tab.
        let mut params = json!({"focused":false});
        if let Some(id) = entry.tab_id {
            params["tabId"] = json!(id);
        } else {
            params["url"] = json!(entry.url);
        }
        Self::send("p:window-create", params)
    }

    fn window_created(&mut self, response: Value) -> OrchStep {
        let Some(id) = response
            .get("id")
            .or_else(|| response.get("windowId"))
            .and_then(Value::as_i64)
        else {
            return Self::error("Undo window creation returned no window ID");
        };
        let original = self.missing_windows[self.window_idx];
        self.window_map.insert(original, id);
        let idx = self
            .entries
            .iter()
            .position(|entry| entry.window_id == original)
            .unwrap();
        if self.entries[idx].tab_id.is_none() {
            let Some(tab_id) = response["tabs"]
                .as_array()
                .and_then(|tabs| tabs.first())
                .and_then(|tab| tab.get("id").or_else(|| tab.get("tabId")))
                .and_then(Value::as_i64)
            else {
                return Self::error("Undo window creation returned no restored tab ID");
            };
            self.entries[idx].tab_id = Some(tab_id);
        }
        self.window_idx += 1;
        self.next_window()
    }

    fn next_entry(&mut self) -> OrchStep {
        if self.entry_idx >= self.entries.len() {
            self.phase = Phase::GroupSnapshot;
            return Self::send("p:snapshot", json!({}));
        }
        let entry = &self.entries[self.entry_idx];
        let window_id = self.window_map[&entry.window_id];
        if let Some(id) = entry.tab_id {
            self.tab_ids.insert(self.entry_idx, id);
            if let Some(pinned) = entry.pinned {
                self.phase = Phase::PrePin;
                Self::send("p:tab-update", json!({"tabId":id,"pinned":pinned}))
            } else {
                self.position_existing()
            }
        } else {
            self.phase = Phase::Create;
            Self::send(
                "p:tab-create",
                json!({
                    "windowId":window_id,"url":entry.url,"index":entry.index,
                    "active":false,"pinned":entry.pinned.unwrap_or(false)
                }),
            )
        }
    }

    fn position_existing(&mut self) -> OrchStep {
        let entry = &self.entries[self.entry_idx];
        let id = self.tab_ids[&self.entry_idx];
        let window_id = self.window_map[&entry.window_id];
        let pin_unchanged = entry.pinned.is_none() || self.pins.get(&id).copied() == entry.pinned;
        if pin_unchanged && self.positions.get(&id) == Some(&(window_id, entry.index)) {
            return self.advance_entry();
        }
        self.phase = Phase::Move;
        Self::send(
            "p:tab-move",
            json!({"tabIds":[id],"windowId":window_id,"index":entry.index}),
        )
    }

    fn after_position(&mut self, response: Value) -> OrchStep {
        let destination = self.window_map[&self.entries[self.entry_idx].window_id];
        let source = self
            .tab_ids
            .get(&self.entry_idx)
            .and_then(|id| self.positions.get(id))
            .map(|position| position.0);
        self.positions
            .retain(|_, position| position.0 != destination && Some(position.0) != source);
        if matches!(self.phase, Phase::Create) {
            let Some(id) = response
                .get("id")
                .or_else(|| response.get("tabId"))
                .and_then(Value::as_i64)
            else {
                return Self::error("Undo tab creation returned no tab ID");
            };
            self.tab_ids.insert(self.entry_idx, id);
        }
        self.advance_entry()
    }

    fn advance_entry(&mut self) -> OrchStep {
        self.restored += 1;
        self.entry_idx += 1;
        self.next_entry()
    }

    fn prepare_groups(&mut self) -> OrchStep {
        for (idx, entry) in self.entries.iter().enumerate() {
            let id = self.tab_ids[&idx];
            let window_id = self.window_map[&entry.window_id];
            if entry.active {
                self.active.push(id);
            }

            if entry.group_id == -1 && entry.group_title.is_none() {
                self.ungrouped.push(id);
                continue;
            }
            if let Some(group) = self.groups.iter_mut().find(|group| {
                group.window_id == window_id
                    && group.original_id == entry.group_id
                    && (entry.group_id != -1 || group.title == entry.group_title)
            }) {
                group.tab_ids.push(id);
            } else {
                self.groups.push(Group {
                    window_id,
                    original_id: entry.group_id,
                    title: entry.group_title.clone(),
                    color: entry.group_color.clone(),
                    collapsed: entry.group_collapsed,
                    tab_ids: vec![id],
                });
            }
        }
        if !self.ungrouped.is_empty() {
            self.phase = Phase::Ungroup;
            return Self::send("p:tab-ungroup", json!({"tabIds":self.ungrouped}));
        }
        self.next_group()
    }

    fn group_snapshot(&mut self, snapshot: Value) -> OrchStep {
        self.existing_groups.clear();
        for window in snapshot["windows"].as_array().into_iter().flatten() {
            if let Some(window_id) = window["windowId"].as_i64() {
                for group in window["groups"].as_array().into_iter().flatten() {
                    if let Some(id) = group["groupId"].as_i64() {
                        self.existing_groups.insert((window_id, id));
                    }
                }
            }
        }
        self.prepare_groups()
    }

    fn next_group(&mut self) -> OrchStep {
        if self.group_idx >= self.groups.len() {
            return self.next_active();
        }
        let group = &self.groups[self.group_idx];
        let mut params = json!({"tabIds":group.tab_ids});
        if self
            .existing_groups
            .contains(&(group.window_id, group.original_id))
        {
            params["groupId"] = json!(group.original_id);
        } else {
            params["createProperties"] = json!({"windowId":group.window_id});
        }
        self.phase = Phase::Group;
        Self::send("p:tab-group", params)
    }

    fn grouped(&mut self, response: Value) -> OrchStep {
        let group = &self.groups[self.group_idx];
        let Some(id) = response
            .get("groupId")
            .and_then(Value::as_i64)
            .or_else(|| response.as_i64())
        else {
            return Self::error("Undo grouping returned no group ID");
        };
        let mut params = json!({"groupId":id});
        if let Some(title) = &group.title {
            params["title"] = json!(title);
        }
        if let Some(color) = &group.color {
            params["color"] = json!(color);
        }
        if let Some(collapsed) = group.collapsed {
            params["collapsed"] = json!(collapsed);
        }
        if params.as_object().unwrap().len() == 1 {
            self.group_idx += 1;
            return self.next_group();
        }
        self.phase = Phase::UpdateGroup;
        Self::send("p:group-update", params)
    }

    fn next_active(&mut self) -> OrchStep {
        if let Some(id) = self.active.pop() {
            self.phase = Phase::Activate;
            return Self::send("p:tab-update", json!({"tabId":id,"active":true}));
        }
        if !self.cleanup.is_empty() {
            self.phase = Phase::Cleanup;
            return Self::send("p:tab-remove", json!({"tabIds":self.cleanup}));
        }
        self.complete()
    }

    fn complete(&self) -> OrchStep {
        OrchStep::Complete {
            response: json!({
                "summary":{"restoredTabs":self.restored,"skippedTabs":self.skipped.len()},
                "skipped":self.skipped,
            }),
            undo: None,
        }
    }
}

impl Orchestration for UndoOrchestration {
    fn recovery_checkpoint(&self) -> Option<Value> {
        if matches!(self.phase, Phase::LegacyGroup) || self.entries.is_empty() {
            return None;
        }
        Some(json!({
            "action":"restore",
            "cleanupTabs":self.cleanup,
            "tabs":self.entries.iter().enumerate().map(|(idx, entry)| json!({
                "tabId":self.tab_ids.get(&idx).copied().or(entry.tab_id),
                "url":entry.url,"pinned":entry.pinned,"active":entry.active,
                "createIfMissing":entry.create_if_missing || entry.tab_id.is_none(),
                "from":{
                    "windowId":self.window_map.get(&entry.window_id).copied().unwrap_or(entry.window_id),
                    "index":entry.index,"groupId":entry.group_id,"groupTitle":entry.group_title,
                    "groupColor":entry.group_color,"groupCollapsed":entry.group_collapsed
                }
            })).collect::<Vec<_>>()
        }))
    }

    fn start(&mut self) -> OrchStep {
        if self.undo["action"].as_str() == Some("group-update") {
            let Some(id) = self.undo["groupId"].as_i64() else {
                return Self::error("Missing groupId in undo record");
            };
            let Some(previous) = self.undo["previous"].as_object() else {
                return Self::error("Previous group values missing");
            };
            let mut params = Map::new();
            for field in ["title", "color", "collapsed"] {
                if let Some(value) = previous.get(field).filter(|value| !value.is_null()) {
                    params.insert(field.into(), value.clone());
                }
            }

            if params.is_empty() {
                return Self::error("No previous values in undo record");
            }
            params.insert("groupId".into(), json!(id));
            self.phase = Phase::LegacyGroup;
            return Self::send("p:group-update", Value::Object(params));
        }
        match normalize(&self.undo) {
            Ok(entries) => self.entries = entries,
            Err(message) => return Self::error(message),
        }
        self.phase = Phase::Snapshot;
        Self::send("p:snapshot", json!({}))
    }

    fn step(&mut self, response: Value) -> OrchStep {
        match self.phase {
            Phase::Snapshot => self.snapshot(response),
            Phase::Window => self.window_created(response),
            Phase::Create | Phase::Move => self.after_position(response),
            Phase::PrePin => self.position_existing(),
            Phase::GroupSnapshot => self.group_snapshot(response),
            Phase::Ungroup => self.next_group(),
            Phase::Group => self.grouped(response),
            Phase::UpdateGroup => {
                self.group_idx += 1;
                self.next_group()
            }
            Phase::Activate => self.next_active(),
            Phase::Cleanup => {
                self.cleanup.clear();
                self.complete()
            }
            Phase::LegacyGroup => OrchStep::Complete {
                response: json!({"summary":{"restoredGroups":1}}),
                undo: None,
            },
        }
    }
}
