use serde_json::{Map, Value};

use super::resolve::resolve_window_id;
use super::scope::select_tabs_by_scope;
use super::OrchStep;

/// Orchestration for the `merge-window` command.
///
/// p:snapshot → plan merge (fromWindow → toWindow) →
/// for each source group: p:tab-move → p:tab-group → p:group-update →
/// for ungrouped: p:tab-move →
/// if closeSource: p:tab-query → p:window-remove → Complete with undo.
#[derive(Debug)]
pub(crate) struct MergeWindowOrchestration {
    params: Value,
    phase: Phase,
    state: Option<MergeState>,
}

#[derive(Debug)]
struct MergeState {
    from_window_id: i64,
    to_window_id: i64,
    close_source: bool,
    batches: Vec<MergeBatch>,
    batch_idx: usize,
    ungrouped_tab_ids: Vec<i64>,
    current_group_id: Option<i64>,
    undo_tabs: Vec<Value>,
    source_closed: bool,
}

#[derive(Debug)]
struct MergeBatch {
    tab_ids: Vec<i64>,
    title: Option<String>,
    color: Option<String>,
    collapsed: Option<bool>,
}

#[derive(Debug)]
enum Phase {
    GetSnapshot,
    MoveBatch,
    GroupBatch,
    UpdateBatch,
    MoveUngrouped,
    QuerySource,
    RemoveSource,
}

impl MergeWindowOrchestration {
    pub(crate) fn new(params: &Value) -> Self {
        Self {
            params: params.clone(),
            phase: Phase::GetSnapshot,
            state: None,
        }
    }
}

impl super::Orchestration for MergeWindowOrchestration {
    fn start(&mut self) -> OrchStep {
        OrchStep::SendPrimitive {
            action: "p:snapshot".to_string(),
            params: Value::Object(Map::new()),
        }
    }

    fn step(&mut self, response: Value) -> OrchStep {
        match self.phase {
            Phase::GetSnapshot => self.handle_snapshot(response),
            Phase::MoveBatch => self.handle_batch_moved(),
            Phase::GroupBatch => self.handle_batch_grouped(response),
            Phase::UpdateBatch => self.handle_batch_updated(),
            Phase::MoveUngrouped => self.after_ungrouped(),
            Phase::QuerySource => self.handle_source_queried(response),
            Phase::RemoveSource => self.complete(),
        }
    }
}

impl MergeWindowOrchestration {
    fn handle_snapshot(&mut self, snapshot: Value) -> OrchStep {
        let from_window_id = match self
            .params
            .get("fromWindowId")
            .and_then(|v| resolve_window_id(&snapshot, v))
        {
            Some(id) => id,
            None => {
                return OrchStep::Error {
                    message: "Missing or invalid fromWindowId".to_string(),
                    hint: None,
                }
            }
        };

        let to_window_id = match self
            .params
            .get("toWindowId")
            .and_then(|v| resolve_window_id(&snapshot, v))
        {
            Some(id) => id,
            None => {
                return OrchStep::Error {
                    message: "Missing or invalid toWindowId".to_string(),
                    hint: None,
                }
            }
        };

        if from_window_id == to_window_id {
            return OrchStep::Error {
                message: "Source and target window are the same".to_string(),
                hint: None,
            };
        }

        let close_source = self
            .params
            .get("closeSource")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        // Get all tabs from source window
        let from_tabs =
            select_tabs_by_scope(&snapshot, &serde_json::json!({"windowId": from_window_id})).tabs;

        if from_tabs.is_empty() {
            return OrchStep::Complete {
                response: serde_json::json!({
                    "fromWindowId": from_window_id,
                    "toWindowId": to_window_id,
                    "sourceClosed": false,
                    "summary": { "movedTabs": 0, "movedGroups": 0 },
                }),
                undo: Some(serde_json::json!({
                    "action": "merge-window",
                    "fromWindowId": from_window_id,
                    "toWindowId": to_window_id,
                    "closedSource": false,
                    "tabs": [],
                })),
            };
        }

        let group_index = build_group_index(&snapshot);

        // Build undo tabs for all source tabs
        let mut undo_tabs: Vec<Value> = Vec::new();
        for tab in &from_tabs {
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

            undo_tabs.push(serde_json::json!({
                "tabId": tab.tab_id,
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
            }));
        }

        // Build batches from source groups
        let mut group_ids: Vec<i64> = Vec::new();
        for t in &from_tabs {
            if t.group_id != -1 && !group_ids.contains(&t.group_id) {
                group_ids.push(t.group_id);
            }
        }

        let mut batches: Vec<MergeBatch> = Vec::new();
        for gid in &group_ids {
            let tab_ids: Vec<i64> = from_tabs
                .iter()
                .filter(|t| t.group_id == *gid)
                .map(|t| t.tab_id)
                .collect();
            let (title, color, collapsed) =
                group_index.get(gid).cloned().unwrap_or((None, None, None));
            batches.push(MergeBatch {
                tab_ids,
                title,
                color,
                collapsed,
            });
        }

        let ungrouped_tab_ids: Vec<i64> = from_tabs
            .iter()
            .filter(|t| t.group_id == -1)
            .map(|t| t.tab_id)
            .collect();

        self.state = Some(MergeState {
            from_window_id,
            to_window_id,
            close_source,
            batches,
            batch_idx: 0,
            ungrouped_tab_ids,
            current_group_id: None,
            undo_tabs,
            source_closed: false,
        });

        self.start_next_batch()
    }

    fn start_next_batch(&mut self) -> OrchStep {
        let state = self.state.as_ref().unwrap();
        if state.batch_idx >= state.batches.len() {
            return self.after_all_batches();
        }

        let batch = &state.batches[state.batch_idx];
        self.phase = Phase::MoveBatch;
        OrchStep::SendPrimitive {
            action: "p:tab-move".to_string(),
            params: serde_json::json!({
                "tabIds": batch.tab_ids,
                "windowId": state.to_window_id,
                "index": -1,
            }),
        }
    }

    fn handle_batch_moved(&mut self) -> OrchStep {
        let state = self.state.as_ref().unwrap();
        let batch = &state.batches[state.batch_idx];

        self.phase = Phase::GroupBatch;
        OrchStep::SendPrimitive {
            action: "p:tab-group".to_string(),
            params: serde_json::json!({
                "tabIds": batch.tab_ids,
                "createProperties": {"windowId": state.to_window_id},
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
        if let Some(title) = &batch.title {
            update.insert("title".to_string(), Value::String(title.clone()));
        }
        if let Some(color) = &batch.color {
            update.insert("color".to_string(), Value::String(color.clone()));
        }
        if let Some(collapsed) = batch.collapsed {
            update.insert("collapsed".to_string(), Value::Bool(collapsed));
        }

        if !update.is_empty() {
            if let Some(gid) = group_id {
                self.phase = Phase::UpdateBatch;
                update.insert("groupId".to_string(), serde_json::json!(gid));
                return OrchStep::SendPrimitive {
                    action: "p:group-update".to_string(),
                    params: Value::Object(update),
                };
            }
        }

        self.advance_batch()
    }

    fn handle_batch_updated(&mut self) -> OrchStep {
        self.advance_batch()
    }

    fn advance_batch(&mut self) -> OrchStep {
        if let Some(state) = self.state.as_mut() {
            state.batch_idx += 1;
        }
        self.start_next_batch()
    }

    fn after_all_batches(&mut self) -> OrchStep {
        let state = self.state.as_ref().unwrap();
        if !state.ungrouped_tab_ids.is_empty() {
            self.phase = Phase::MoveUngrouped;
            OrchStep::SendPrimitive {
                action: "p:tab-move".to_string(),
                params: serde_json::json!({
                    "tabIds": state.ungrouped_tab_ids,
                    "windowId": state.to_window_id,
                    "index": -1,
                }),
            }
        } else {
            self.after_ungrouped()
        }
    }

    fn after_ungrouped(&mut self) -> OrchStep {
        let state = self.state.as_ref().unwrap();
        if state.close_source {
            self.phase = Phase::QuerySource;
            OrchStep::SendPrimitive {
                action: "p:tab-query".to_string(),
                params: serde_json::json!({"query": {"windowId": state.from_window_id}}),
            }
        } else {
            self.complete()
        }
    }

    fn handle_source_queried(&mut self, response: Value) -> OrchStep {
        let remaining = response.as_array().map(|a| a.len()).unwrap_or(0);
        if remaining == 0 {
            if let Some(state) = self.state.as_mut() {
                state.source_closed = true;
            }
            let state = self.state.as_ref().unwrap();
            self.phase = Phase::RemoveSource;
            OrchStep::SendPrimitive {
                action: "p:window-remove".to_string(),
                params: serde_json::json!({"windowId": state.from_window_id}),
            }
        } else {
            self.complete()
        }
    }

    fn complete(&self) -> OrchStep {
        let state = self.state.as_ref().unwrap();
        let total_tabs: usize = state.batches.iter().map(|b| b.tab_ids.len()).sum::<usize>()
            + state.ungrouped_tab_ids.len();

        OrchStep::Complete {
            response: serde_json::json!({
                "fromWindowId": state.from_window_id,
                "toWindowId": state.to_window_id,
                "sourceClosed": state.source_closed,
                "summary": {
                    "movedTabs": total_tabs,
                    "movedGroups": state.batches.len(),
                },
            }),
            undo: Some(serde_json::json!({
                "action": "merge-window",
                "fromWindowId": state.from_window_id,
                "toWindowId": state.to_window_id,
                "closedSource": state.source_closed,
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
            }, {
                "windowId": 200,
                "focused": false,
                "tabs": [
                    {"tabId": 3, "windowId": 200, "index": 0, "url": "https://c.com", "title": "C", "active": true, "pinned": false, "groupId": -1}
                ],
                "groups": []
            }]
        })
    }

    #[test]
    fn merge_window_moves_groups_and_ungrouped() {
        let params = serde_json::json!({"fromWindowId": 100, "toWindowId": 200});
        let mut orch = MergeWindowOrchestration::new(&params);

        let step = orch.start();
        assert!(matches!(&step, OrchStep::SendPrimitive { action, .. } if action == "p:snapshot"));

        // Snapshot → move first batch (group "Dev")
        let step = orch.step(snapshot());
        let OrchStep::SendPrimitive { action, params } = &step else {
            panic!("expected tab-move, got {step:?}");
        };
        assert_eq!(action, "p:tab-move");
        assert_eq!(params["tabIds"], serde_json::json!([1]));
        assert_eq!(params["windowId"], 200);

        // Moved → group
        let step = orch.step(serde_json::json!({}));
        assert!(matches!(&step, OrchStep::SendPrimitive { action, .. } if action == "p:tab-group"));

        // Grouped → update with title "Dev"
        let step = orch.step(serde_json::json!({"groupId": 50}));
        let OrchStep::SendPrimitive { action, params } = &step else {
            panic!("expected group-update, got {step:?}");
        };
        assert_eq!(action, "p:group-update");
        assert_eq!(params["title"], "Dev");

        // Updated → move ungrouped tabs
        let step = orch.step(serde_json::json!({}));
        let OrchStep::SendPrimitive { action, params } = &step else {
            panic!("expected tab-move for ungrouped, got {step:?}");
        };
        assert_eq!(action, "p:tab-move");
        assert_eq!(params["tabIds"], serde_json::json!([2]));

        // Ungrouped moved → Complete (no closeSource)
        let step = orch.step(serde_json::json!({}));
        let OrchStep::Complete { response, undo } = step else {
            panic!("expected Complete");
        };
        assert_eq!(response["summary"]["movedTabs"], 2);
        assert_eq!(response["summary"]["movedGroups"], 1);
        assert_eq!(response["sourceClosed"], false);

        let undo = undo.unwrap();
        assert_eq!(undo["action"], "merge-window");
        assert_eq!(undo["tabs"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn merge_window_close_source() {
        let params =
            serde_json::json!({"fromWindowId": 100, "toWindowId": 200, "closeSource": true});
        let mut orch = MergeWindowOrchestration::new(&params);
        let _ = orch.start();

        // Advance through: snapshot → move batch → group → update → move ungrouped
        let _ = orch.step(snapshot()); // → tab-move (batch)
        let _ = orch.step(serde_json::json!({})); // → tab-group
        let _ = orch.step(serde_json::json!({"groupId": 50})); // → group-update
        let _ = orch.step(serde_json::json!({})); // → tab-move (ungrouped)

        // After ungrouped → query source window
        let step = orch.step(serde_json::json!({}));
        let OrchStep::SendPrimitive { action, .. } = &step else {
            panic!("expected tab-query, got {step:?}");
        };
        assert_eq!(action, "p:tab-query");

        // Source empty → remove window
        let step = orch.step(serde_json::json!([]));
        let OrchStep::SendPrimitive { action, params } = &step else {
            panic!("expected window-remove, got {step:?}");
        };
        assert_eq!(action, "p:window-remove");
        assert_eq!(params["windowId"], 100);

        // Removed → Complete
        let step = orch.step(serde_json::json!({}));
        let OrchStep::Complete { response, undo } = step else {
            panic!("expected Complete");
        };
        assert_eq!(response["sourceClosed"], true);
        let undo = undo.unwrap();
        assert_eq!(undo["closedSource"], true);
    }

    #[test]
    fn merge_window_same_window_errors() {
        let params = serde_json::json!({"fromWindowId": 100, "toWindowId": 100});
        let mut orch = MergeWindowOrchestration::new(&params);
        let _ = orch.start();
        let step = orch.step(snapshot());
        assert!(matches!(&step, OrchStep::Error { message, .. } if message.contains("same")));
    }

    #[test]
    fn merge_window_missing_params_errors() {
        let params = serde_json::json!({});
        let mut orch = MergeWindowOrchestration::new(&params);
        let _ = orch.start();
        let step = orch.step(snapshot());
        assert!(
            matches!(&step, OrchStep::Error { message, .. } if message.contains("fromWindowId"))
        );
    }
}
