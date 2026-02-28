use serde_json::{Map, Value};

use super::resolve::{resolve_group, resolve_window_id};
use super::OrchStep;

/// Orchestration for the `group-assign` command.
///
/// Multi-step: p:snapshot → resolve target → p:tab-move (if cross-window) →
/// p:tab-group → p:group-update (if created) → Complete with undo.
#[derive(Debug)]
pub(crate) struct GroupAssignOrchestration {
    params: Value,
    phase: Phase,
    state: Option<AssignState>,
}

#[derive(Debug)]
struct AssignState {
    resolved_tab_ids: Vec<i64>,
    target_group_id: Option<i64>,
    target_window_id: i64,
    target_title: Option<String>,
    created: bool,
    move_ids: Vec<i64>,
    skipped: Vec<Value>,
    undo_tabs: Vec<Value>,
}

#[derive(Debug)]
enum Phase {
    GetSnapshot,
    MoveTabs,
    GroupTabs,
    UpdateGroup,
}

impl GroupAssignOrchestration {
    pub(crate) fn new(params: &Value) -> Self {
        Self {
            params: params.clone(),
            phase: Phase::GetSnapshot,
            state: None,
        }
    }
}

impl super::Orchestration for GroupAssignOrchestration {
    fn start(&mut self) -> OrchStep {
        OrchStep::SendPrimitive {
            action: "p:snapshot".to_string(),
            params: Value::Object(Map::new()),
        }
    }

    fn step(&mut self, response: Value) -> OrchStep {
        match self.phase {
            Phase::GetSnapshot => self.handle_snapshot(response),
            Phase::MoveTabs => self.handle_moved(response),
            Phase::GroupTabs => self.handle_grouped(response),
            Phase::UpdateGroup => self.handle_updated(),
        }
    }
}

impl GroupAssignOrchestration {
    fn handle_snapshot(&mut self, snapshot: Value) -> OrchStep {
        let raw_tab_ids: Vec<i64> = self
            .params
            .get("tabIds")
            .and_then(Value::as_array)
            .map(|arr| arr.iter().filter_map(Value::as_i64).collect())
            .unwrap_or_default();

        if raw_tab_ids.is_empty() {
            return OrchStep::Error {
                message: "Missing tabIds".to_string(),
                hint: None,
            };
        }

        let group_id_param = self.params.get("groupId").and_then(Value::as_i64);
        let group_title = self
            .params
            .get("groupTitle")
            .and_then(Value::as_str)
            .map(|s| s.trim().to_string());

        if group_id_param.is_none() && group_title.as_deref().map_or(true, str::is_empty) {
            return OrchStep::Error {
                message: "Missing group identifier".to_string(),
                hint: None,
            };
        }

        let window_id_param = self
            .params
            .get("windowId")
            .and_then(|v| resolve_window_id(&snapshot, v));

        // Build tab index from snapshot
        let mut tab_index: std::collections::HashMap<i64, (Value, i64)> =
            std::collections::HashMap::new();
        if let Some(windows) = snapshot.get("windows").and_then(Value::as_array) {
            for win in windows {
                let win_id = win.get("windowId").and_then(Value::as_i64).unwrap_or(0);
                if let Some(tabs) = win.get("tabs").and_then(Value::as_array) {
                    for tab in tabs {
                        if let Some(tid) = tab.get("tabId").and_then(Value::as_i64) {
                            tab_index.insert(tid, (tab.clone(), win_id));
                        }
                    }
                }
            }
        }

        let mut resolved_tab_ids = Vec::new();
        let mut source_windows = std::collections::HashSet::new();
        let mut skipped = Vec::new();
        let mut undo_tabs = Vec::new();

        for tab_id in &raw_tab_ids {
            if let Some((tab, win_id)) = tab_index.get(tab_id) {
                resolved_tab_ids.push(*tab_id);
                source_windows.insert(*win_id);
                undo_tabs.push(serde_json::json!({
                    "tabId": tab_id,
                    "windowId": win_id,
                    "index": tab.get("index"),
                    "groupId": tab.get("groupId"),
                    "groupTitle": tab.get("groupTitle"),
                    "groupColor": tab.get("groupColor"),
                    "groupCollapsed": tab.get("groupCollapsed"),
                }));
            } else {
                skipped.push(serde_json::json!({"tabId": tab_id, "reason": "not_found"}));
            }
        }

        if resolved_tab_ids.is_empty() {
            return OrchStep::Error {
                message: "No matching tabs found".to_string(),
                hint: None,
            };
        }

        // Resolve target group
        let mut target_group_id: Option<i64> = None;
        #[allow(unused_assignments)]
        let mut target_window_id: Option<i64> = None;
        #[allow(unused_assignments)]
        let mut target_title: Option<String> = None;
        let mut created = false;

        if let Some(gid) = group_id_param {
            match resolve_group(&snapshot, Some(gid), None, None) {
                Ok(m) => {
                    target_group_id = Some(m.group_id);
                    target_window_id = Some(m.window_id);
                    target_title = m.title;
                    if matches!(window_id_param, Some(w) if w != m.window_id) {
                        return OrchStep::Error {
                            message: "Group is not in the specified window".to_string(),
                            hint: None,
                        };
                    }
                }
                Err(e) => return e,
            }
        } else {
            let gt = group_title.as_deref().unwrap_or("");
            match resolve_group(&snapshot, None, Some(gt), window_id_param) {
                Ok(m) => {
                    target_group_id = Some(m.group_id);
                    target_window_id = Some(m.window_id);
                    target_title = m.title.or_else(|| Some(gt.to_string()));
                }
                Err(OrchStep::Error { message, .. })
                    if message.contains("not found")
                        && self.params.get("create").and_then(Value::as_bool) == Some(true) =>
                {
                    target_window_id = window_id_param.or_else(|| {
                        if source_windows.len() == 1 {
                            source_windows.iter().next().copied()
                        } else {
                            None
                        }
                    });
                    if target_window_id.is_none() {
                        return OrchStep::Error {
                            message:
                                "Multiple source windows. Provide --window to create a new group."
                                    .to_string(),
                            hint: None,
                        };
                    }
                    target_title = Some(gt.to_string());
                    created = true;
                }
                Err(e) => return e,
            }
        }

        let Some(target_window_id) = target_window_id else {
            return OrchStep::Error {
                message: "Target window not found".to_string(),
                hint: None,
            };
        };

        // Find tabs that need cross-window move
        let move_ids: Vec<i64> = resolved_tab_ids
            .iter()
            .filter(|tid| matches!(tab_index.get(tid), Some((_, wid)) if *wid != target_window_id))
            .copied()
            .collect();

        self.state = Some(AssignState {
            resolved_tab_ids,
            target_group_id,
            target_window_id,
            target_title,
            created,
            move_ids: move_ids.clone(),
            skipped,
            undo_tabs,
        });

        if !move_ids.is_empty() {
            self.phase = Phase::MoveTabs;
            OrchStep::SendPrimitive {
                action: "p:tab-move".to_string(),
                params: serde_json::json!({
                    "tabIds": move_ids,
                    "windowId": target_window_id,
                    "index": -1
                }),
            }
        } else {
            self.send_group_step()
        }
    }

    fn handle_moved(&mut self, _response: Value) -> OrchStep {
        self.send_group_step()
    }

    fn send_group_step(&mut self) -> OrchStep {
        let state = self.state.as_ref().unwrap();
        self.phase = Phase::GroupTabs;

        if let Some(gid) = state.target_group_id {
            OrchStep::SendPrimitive {
                action: "p:tab-group".to_string(),
                params: serde_json::json!({
                    "groupId": gid,
                    "tabIds": state.resolved_tab_ids,
                }),
            }
        } else {
            OrchStep::SendPrimitive {
                action: "p:tab-group".to_string(),
                params: serde_json::json!({
                    "tabIds": state.resolved_tab_ids,
                    "createProperties": { "windowId": state.target_window_id },
                }),
            }
        }
    }

    fn handle_grouped(&mut self, response: Value) -> OrchStep {
        let state = self.state.as_mut().unwrap();

        // If we created a new group, update its properties
        if state.created {
            // Response from p:tab-group is the groupId
            let group_id = response
                .get("groupId")
                .and_then(Value::as_i64)
                .or_else(|| response.as_i64());
            if let Some(gid) = group_id {
                state.target_group_id = Some(gid);
            }

            let mut update = Map::new();
            if let Some(title) = &state.target_title {
                update.insert("title".to_string(), Value::String(title.clone()));
            }
            if let Some(color) = self.params.get("color").and_then(Value::as_str) {
                let trimmed = color.trim();
                if !trimmed.is_empty() {
                    update.insert("color".to_string(), Value::String(trimmed.to_string()));
                }
            }
            if let Some(collapsed) = self.params.get("collapsed").and_then(Value::as_bool) {
                update.insert("collapsed".to_string(), Value::Bool(collapsed));
            }

            if !update.is_empty() {
                if let Some(gid) = state.target_group_id {
                    self.phase = Phase::UpdateGroup;
                    return OrchStep::SendPrimitive {
                        action: "p:group-update".to_string(),
                        params: serde_json::json!({
                            "groupId": gid,
                            "updateProperties": Value::Object(update),
                        }),
                    };
                }
            }
        }

        self.handle_updated()
    }

    fn handle_updated(&mut self) -> OrchStep {
        let state = self.state.as_ref().unwrap();

        let color = self
            .params
            .get("color")
            .and_then(Value::as_str)
            .map(|s| s.trim())
            .filter(|s| !s.is_empty());
        let collapsed = self.params.get("collapsed").and_then(Value::as_bool);

        let undo = serde_json::json!({
            "action": "group-assign",
            "groupId": state.target_group_id,
            "groupTitle": state.target_title,
            "groupColor": color,
            "groupCollapsed": collapsed,
            "created": state.created,
            "tabs": state.undo_tabs,
        });

        OrchStep::Complete {
            response: serde_json::json!({
                "groupId": state.target_group_id,
                "windowId": state.target_window_id,
                "created": state.created,
                "summary": {
                    "movedTabs": state.move_ids.len(),
                    "groupedTabs": state.resolved_tab_ids.len(),
                    "skippedTabs": state.skipped.len(),
                },
                "skipped": state.skipped,
            }),
            undo: Some(undo),
        }
    }
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
                    {"tabId": 1, "windowId": 100, "index": 0, "groupId": 10, "groupTitle": "Dev", "groupColor": "blue", "groupCollapsed": false},
                    {"tabId": 2, "windowId": 100, "index": 1, "groupId": -1},
                    {"tabId": 3, "windowId": 100, "index": 2, "groupId": -1}
                ],
                "groups": [
                    {"groupId": 10, "title": "Dev", "color": "blue", "collapsed": false}
                ]
            }, {
                "windowId": 200,
                "focused": false,
                "tabs": [
                    {"tabId": 4, "windowId": 200, "index": 0, "groupId": -1}
                ],
                "groups": []
            }]
        })
    }

    #[test]
    fn assign_to_existing_group_same_window() {
        let params = serde_json::json!({"tabIds": [2, 3], "groupId": 10});
        let mut orch = GroupAssignOrchestration::new(&params);

        let step = orch.start();
        assert!(matches!(&step, OrchStep::SendPrimitive { action, .. } if action == "p:snapshot"));

        // Got snapshot → should go straight to tab-group (no move needed)
        let step = orch.step(snapshot());
        let OrchStep::SendPrimitive { action, params } = &step else {
            panic!("expected tab-group, got {step:?}");
        };
        assert_eq!(action, "p:tab-group");
        assert_eq!(params["groupId"], 10);

        // Grouped → complete
        let step = orch.step(serde_json::json!({"groupId": 10}));
        let OrchStep::Complete { response, undo } = step else {
            panic!("expected Complete");
        };
        assert_eq!(response["summary"]["groupedTabs"], 2);
        assert_eq!(response["summary"]["movedTabs"], 0);

        let undo = undo.unwrap();
        assert_eq!(undo["action"], "group-assign");
        assert_eq!(undo["tabs"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn assign_cross_window_moves_first() {
        let params = serde_json::json!({"tabIds": [4], "groupId": 10});
        let mut orch = GroupAssignOrchestration::new(&params);
        let _ = orch.start();

        let step = orch.step(snapshot());
        let OrchStep::SendPrimitive { action, params } = &step else {
            panic!("expected tab-move, got {step:?}");
        };
        assert_eq!(action, "p:tab-move");
        assert_eq!(params["windowId"], 100);

        // After move → group
        let step = orch.step(serde_json::json!({}));
        assert!(matches!(&step, OrchStep::SendPrimitive { action, .. } if action == "p:tab-group"));
    }

    #[test]
    fn assign_create_new_group() {
        let params = serde_json::json!({"tabIds": [2], "groupTitle": "NewGroup", "create": true, "color": "red"});
        let mut orch = GroupAssignOrchestration::new(&params);
        let _ = orch.start();

        let step = orch.step(snapshot());
        let OrchStep::SendPrimitive { action, params } = &step else {
            panic!("expected tab-group, got {step:?}");
        };
        assert_eq!(action, "p:tab-group");
        assert!(params.get("createProperties").is_some());

        // After group → should update with title/color
        let step = orch.step(serde_json::json!({"groupId": 99}));
        let OrchStep::SendPrimitive { action, params } = &step else {
            panic!("expected group-update, got {step:?}");
        };
        assert_eq!(action, "p:group-update");
        assert_eq!(params["updateProperties"]["title"], "NewGroup");
        assert_eq!(params["updateProperties"]["color"], "red");
    }

    #[test]
    fn assign_missing_tab_ids_errors() {
        let params = serde_json::json!({"groupId": 10});
        let mut orch = GroupAssignOrchestration::new(&params);
        let _ = orch.start();
        let step = orch.step(snapshot());
        assert!(
            matches!(&step, OrchStep::Error { message, .. } if message.contains("Missing tabIds"))
        );
    }
}
