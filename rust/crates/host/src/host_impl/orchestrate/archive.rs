use serde_json::{Map, Value};

use super::scope::select_tabs_by_scope;
use super::OrchStep;

/// Orchestration for the `archive` command.
///
/// p:snapshot → select tabs by scope → plan archive batches →
/// p:window-create → for each batch: p:tab-move → p:tab-group →
/// p:group-update → Complete with undo.
///
/// Batches are organized by source window and group, labelled as
/// "W1 - GroupName" or "W1 - Ungrouped".
#[derive(Debug)]
pub(crate) struct ArchiveOrchestration {
    params: Value,
    phase: Phase,
    state: Option<ArchiveState>,
}

#[derive(Debug)]
struct ArchiveState {
    archive_window_id: Option<i64>,
    batches: Vec<ArchiveBatch>,
    batch_idx: usize,
    current_group_id: Option<i64>,
    undo_tabs: Vec<Value>,
}

#[derive(Debug)]
struct ArchiveBatch {
    label: String,
    tab_ids: Vec<i64>,
    color: Option<String>,
}

#[derive(Debug)]
enum Phase {
    GetSnapshot,
    CreateArchiveWindow,
    MoveBatch,
    GroupBatch,
    UpdateBatch,
}

impl ArchiveOrchestration {
    pub(crate) fn new(params: &Value) -> Self {
        Self {
            params: params.clone(),
            phase: Phase::GetSnapshot,
            state: None,
        }
    }
}

impl super::Orchestration for ArchiveOrchestration {
    fn start(&mut self) -> OrchStep {
        OrchStep::SendPrimitive {
            action: "p:snapshot".to_string(),
            params: Value::Object(Map::new()),
        }
    }

    fn step(&mut self, response: Value) -> OrchStep {
        match self.phase {
            Phase::GetSnapshot => self.handle_snapshot(response),
            Phase::CreateArchiveWindow => self.handle_archive_window_created(response),
            Phase::MoveBatch => self.handle_batch_moved(),
            Phase::GroupBatch => self.handle_batch_grouped(response),
            Phase::UpdateBatch => self.handle_batch_updated(),
        }
    }
}

impl ArchiveOrchestration {
    fn handle_snapshot(&mut self, snapshot: Value) -> OrchStep {
        let scope_result = select_tabs_by_scope(&snapshot, &self.params);
        if let Some(err) = scope_result.error {
            return OrchStep::Error {
                message: err,
                hint: None,
            };
        }

        let tabs = scope_result.tabs;
        if tabs.is_empty() {
            return OrchStep::Complete {
                response: serde_json::json!({
                    "summary": { "archivedTabs": 0, "archivedGroups": 0 },
                }),
                undo: Some(serde_json::json!({
                    "action": "archive",
                    "tabs": [],
                })),
            };
        }

        let group_index = build_group_index(&snapshot);

        // Collect unique window IDs in order
        let mut window_ids: Vec<i64> = Vec::new();
        for tab in &tabs {
            if !window_ids.contains(&tab.window_id) {
                window_ids.push(tab.window_id);
            }
        }

        let mut batches: Vec<ArchiveBatch> = Vec::new();
        let mut undo_tabs: Vec<Value> = Vec::new();

        for (win_idx, &win_id) in window_ids.iter().enumerate() {
            let win_label = format!("W{}", win_idx + 1);
            let win_tabs: Vec<_> = tabs.iter().filter(|t| t.window_id == win_id).collect();

            // Collect unique group IDs in this window (preserving order)
            let mut group_ids: Vec<i64> = Vec::new();
            for t in &win_tabs {
                if t.group_id != -1 && !group_ids.contains(&t.group_id) {
                    group_ids.push(t.group_id);
                }
            }

            // One batch per source group
            for gid in &group_ids {
                let group_tabs: Vec<_> = win_tabs.iter().filter(|t| t.group_id == *gid).collect();
                let (title, color, _collapsed) =
                    group_index.get(gid).cloned().unwrap_or((None, None, None));
                let label_title = title.unwrap_or_else(|| format!("Group {gid}"));
                let label = format!("{win_label} - {label_title}");

                for t in &group_tabs {
                    undo_tabs.push(build_undo_tab(t, &group_index));
                }

                batches.push(ArchiveBatch {
                    label,
                    tab_ids: group_tabs.iter().map(|t| t.tab_id).collect(),
                    color,
                });
            }

            // Ungrouped tabs batch
            let ungrouped: Vec<_> = win_tabs.iter().filter(|t| t.group_id == -1).collect();
            if !ungrouped.is_empty() {
                let label = format!("{win_label} - Ungrouped");
                for t in &ungrouped {
                    undo_tabs.push(build_undo_tab(t, &group_index));
                }
                batches.push(ArchiveBatch {
                    label,
                    tab_ids: ungrouped.iter().map(|t| t.tab_id).collect(),
                    color: None,
                });
            }
        }

        self.state = Some(ArchiveState {
            archive_window_id: self.params.get("archiveWindowId").and_then(Value::as_i64),
            batches,
            batch_idx: 0,
            current_group_id: None,
            undo_tabs,
        });

        let state = self.state.as_ref().unwrap();
        if state.archive_window_id.is_some() {
            self.start_next_batch()
        } else {
            self.phase = Phase::CreateArchiveWindow;
            OrchStep::SendPrimitive {
                action: "p:window-create".to_string(),
                params: serde_json::json!({"focused": false}),
            }
        }
    }

    fn handle_archive_window_created(&mut self, response: Value) -> OrchStep {
        let window_id = response
            .get("id")
            .or_else(|| response.get("windowId"))
            .and_then(Value::as_i64);

        let Some(window_id) = window_id else {
            return OrchStep::Error {
                message: "Failed to create archive window".to_string(),
                hint: None,
            };
        };

        if let Some(state) = self.state.as_mut() {
            state.archive_window_id = Some(window_id);
        }

        self.start_next_batch()
    }

    fn start_next_batch(&mut self) -> OrchStep {
        let state = self.state.as_ref().unwrap();
        if state.batch_idx >= state.batches.len() {
            return self.complete();
        }

        let batch = &state.batches[state.batch_idx];
        let archive_window_id = state.archive_window_id.unwrap();

        self.phase = Phase::MoveBatch;
        OrchStep::SendPrimitive {
            action: "p:tab-move".to_string(),
            params: serde_json::json!({
                "tabIds": batch.tab_ids,
                "windowId": archive_window_id,
                "index": -1,
            }),
        }
    }

    fn handle_batch_moved(&mut self) -> OrchStep {
        let state = self.state.as_ref().unwrap();
        let batch = &state.batches[state.batch_idx];
        let archive_window_id = state.archive_window_id.unwrap();

        self.phase = Phase::GroupBatch;
        OrchStep::SendPrimitive {
            action: "p:tab-group".to_string(),
            params: serde_json::json!({
                "tabIds": batch.tab_ids,
                "createProperties": {"windowId": archive_window_id},
            }),
        }
    }

    fn handle_batch_grouped(&mut self, response: Value) -> OrchStep {
        let group_id = response
            .get("groupId")
            .and_then(Value::as_i64)
            .or_else(|| response.as_i64());

        if let Some(state) = self.state.as_mut() {
            state.current_group_id = group_id;
        }

        let state = self.state.as_ref().unwrap();
        let batch = &state.batches[state.batch_idx];

        let mut update = Map::new();
        update.insert("title".to_string(), Value::String(batch.label.clone()));
        if let Some(color) = &batch.color {
            update.insert("color".to_string(), Value::String(color.clone()));
        }
        update.insert("collapsed".to_string(), Value::Bool(true));

        self.phase = Phase::UpdateBatch;
        update.insert(
            "groupId".to_string(),
            serde_json::json!(state.current_group_id),
        );
        OrchStep::SendPrimitive {
            action: "p:group-update".to_string(),
            params: Value::Object(update),
        }
    }

    fn handle_batch_updated(&mut self) -> OrchStep {
        if let Some(state) = self.state.as_mut() {
            state.batch_idx += 1;
        }
        self.start_next_batch()
    }

    fn complete(&self) -> OrchStep {
        let state = self.state.as_ref().unwrap();
        let total_tabs: usize = state.batches.iter().map(|b| b.tab_ids.len()).sum();

        OrchStep::Complete {
            response: serde_json::json!({
                "archiveWindowId": state.archive_window_id,
                "summary": {
                    "archivedTabs": total_tabs,
                    "archivedGroups": state.batches.len(),
                    "movedTabs": total_tabs,
                },
            }),
            undo: Some(serde_json::json!({
                "action": "archive",
                "tabs": state.undo_tabs,
            })),
        }
    }
}

type GroupInfo = (Option<String>, Option<String>, Option<bool>);

fn build_group_index(snapshot: &Value) -> std::collections::HashMap<i64, GroupInfo> {
    let mut idx = std::collections::HashMap::new();
    if let Some(windows) = snapshot.get("windows").and_then(Value::as_array) {
        for win in windows {
            if let Some(groups) = win.get("groups").and_then(Value::as_array) {
                for g in groups {
                    if let Some(gid) = g.get("groupId").and_then(Value::as_i64) {
                        idx.insert(
                            gid,
                            (
                                g.get("title").and_then(Value::as_str).map(String::from),
                                g.get("color").and_then(Value::as_str).map(String::from),
                                g.get("collapsed").and_then(Value::as_bool),
                            ),
                        );
                    }
                }
            }
        }
    }
    idx
}

fn build_undo_tab(
    tab: &super::scope::ScopedTab,
    group_index: &std::collections::HashMap<i64, GroupInfo>,
) -> Value {
    let (group_title, group_color, group_collapsed) = if tab.group_id != -1 {
        group_index
            .get(&tab.group_id)
            .map(|g| (g.0.clone(), g.1.clone(), g.2))
            .unwrap_or((
                tab.group_title.clone(),
                tab.group_color.clone(),
                tab.group_collapsed,
            ))
    } else {
        (None, None, None)
    };

    serde_json::json!({
        "url": tab.url,
        "title": tab.title,
        "pinned": tab.pinned,
        "active": tab.active,
        "from": {
            "windowId": tab.window_id,
            "index": tab.index,
            "groupId": tab.group_id,
            "groupTitle": group_title,
            "groupColor": group_color,
            "groupCollapsed": group_collapsed,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host_impl::orchestrate::Orchestration;

    fn snapshot() -> Value {
        serde_json::json!({
            "windows": [{
                "windowId": 100,
                "focused": true,
                "tabs": [
                    {"tabId": 1, "windowId": 100, "index": 0, "url": "https://a.com", "title": "A", "active": true, "pinned": false, "groupId": 10, "groupTitle": "Dev", "groupColor": "blue", "groupCollapsed": false},
                    {"tabId": 2, "windowId": 100, "index": 1, "url": "https://b.com", "title": "B", "active": false, "pinned": false, "groupId": -1}
                ],
                "groups": [
                    {"groupId": 10, "title": "Dev", "color": "blue", "collapsed": false}
                ]
            }]
        })
    }

    #[test]
    fn archive_creates_window_and_processes_batches() {
        let params = serde_json::json!({"windowId": 100});
        let mut orch = ArchiveOrchestration::new(&params);

        let step = orch.start();
        assert!(matches!(&step, OrchStep::SendPrimitive { action, .. } if action == "p:snapshot"));

        // Snapshot → create archive window
        let step = orch.step(snapshot());
        assert!(
            matches!(&step, OrchStep::SendPrimitive { action, .. } if action == "p:window-create")
        );

        // Window created → move first batch (grouped tabs)
        let step = orch.step(serde_json::json!({"id": 300}));
        let OrchStep::SendPrimitive { action, params } = &step else {
            panic!("expected tab-move, got {step:?}");
        };
        assert_eq!(action, "p:tab-move");
        assert_eq!(params["tabIds"], serde_json::json!([1]));
        assert_eq!(params["windowId"], 300);

        // Moved → group
        let step = orch.step(serde_json::json!({}));
        assert!(matches!(&step, OrchStep::SendPrimitive { action, .. } if action == "p:tab-group"));

        // Grouped → update with label "W1 - Dev"
        let step = orch.step(serde_json::json!({"groupId": 50}));
        let OrchStep::SendPrimitive { action, params } = &step else {
            panic!("expected group-update, got {step:?}");
        };
        assert_eq!(action, "p:group-update");
        assert_eq!(params["title"], "W1 - Dev");
        assert_eq!(params["collapsed"], true);

        // Updated → move second batch (ungrouped)
        let step = orch.step(serde_json::json!({}));
        let OrchStep::SendPrimitive { action, params } = &step else {
            panic!("expected tab-move for ungrouped, got {step:?}");
        };
        assert_eq!(action, "p:tab-move");
        assert_eq!(params["tabIds"], serde_json::json!([2]));

        // Moved → group ungrouped
        let step = orch.step(serde_json::json!({}));
        assert!(matches!(&step, OrchStep::SendPrimitive { action, .. } if action == "p:tab-group"));

        // Grouped → update with "W1 - Ungrouped"
        let step = orch.step(serde_json::json!({"groupId": 51}));
        let OrchStep::SendPrimitive { action, params } = &step else {
            panic!("expected group-update, got {step:?}");
        };
        assert_eq!(action, "p:group-update");
        assert_eq!(params["title"], "W1 - Ungrouped");

        // Updated → Complete
        let step = orch.step(serde_json::json!({}));
        let OrchStep::Complete { response, undo } = step else {
            panic!("expected Complete");
        };
        assert_eq!(response["summary"]["archivedTabs"], 2);
        assert_eq!(response["summary"]["archivedGroups"], 2);
        assert_eq!(response["archiveWindowId"], 300);

        let undo = undo.unwrap();
        assert_eq!(undo["action"], "archive");
        assert_eq!(undo["tabs"].as_array().unwrap().len(), 2);
        assert_eq!(undo["tabs"][0]["from"]["groupTitle"], "Dev");
    }

    #[test]
    fn archive_empty_scope_returns_zero() {
        let params = serde_json::json!({"windowId": 999});
        let mut orch = ArchiveOrchestration::new(&params);
        let _ = orch.start();

        let step = orch.step(snapshot());
        let OrchStep::Complete { response, .. } = step else {
            panic!("expected Complete for empty scope");
        };
        assert_eq!(response["summary"]["archivedTabs"], 0);
    }

    #[test]
    fn archive_uses_existing_window() {
        let params = serde_json::json!({"windowId": 100, "archiveWindowId": 500});
        let mut orch = ArchiveOrchestration::new(&params);
        let _ = orch.start();

        // Snapshot → skip window-create, go directly to tab-move
        let step = orch.step(snapshot());
        let OrchStep::SendPrimitive { action, params } = &step else {
            panic!("expected tab-move, got {step:?}");
        };
        assert_eq!(action, "p:tab-move");
        assert_eq!(params["windowId"], 500);
    }
}
