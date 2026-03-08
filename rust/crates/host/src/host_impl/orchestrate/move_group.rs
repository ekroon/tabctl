use serde_json::{Map, Value};

use super::resolve::{resolve_group, resolve_window_id, window_is_incognito};
use super::OrchStep;

/// Find a tab's index in the snapshot.
fn find_tab_index(snapshot: &Value, tab_id: i64) -> Option<i64> {
    snapshot
        .get("windows")
        .and_then(Value::as_array)?
        .iter()
        .flat_map(|w| {
            w.get("tabs")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .find(|t| t.get("tabId").and_then(Value::as_i64) == Some(tab_id))
        .and_then(|t| t.get("index").and_then(Value::as_i64))
}

/// Orchestration for the `move-group` command.
///
/// p:snapshot → resolve source group → p:tab-move or p:window-create →
/// p:tab-group → p:group-update → Complete with undo.
///
/// Same-window moves only reindex tabs (no re-grouping).
/// Cross-window and new-window moves create a new group in the target.
#[derive(Debug)]
pub(crate) struct MoveGroupOrchestration {
    params: Value,
    phase: Phase,
    state: Option<MoveGroupState>,
}

#[derive(Debug)]
struct MoveGroupState {
    source_group_id: i64,
    source_window_id: i64,
    has_incognito: bool,
    group_title: Option<String>,
    group_color: Option<String>,
    group_collapsed: Option<bool>,
    tab_ids: Vec<i64>,
    undo_tabs: Vec<Value>,
    target_window_id: Option<i64>,
    new_group_id: Option<i64>,
    needs_regroup: bool,
    verify_attempts: u8,
}

#[derive(Debug)]
enum Phase {
    GetSnapshot,
    CreateWindow,
    MoveRemaining,
    MoveTabs,
    GroupTabs,
    UpdateGroup,
    Verify,
    RegroupStragglers,
}

impl MoveGroupOrchestration {
    pub(crate) fn new(params: &Value) -> Self {
        Self {
            params: params.clone(),
            phase: Phase::GetSnapshot,
            state: None,
        }
    }
}

impl super::Orchestration for MoveGroupOrchestration {
    fn start(&mut self) -> OrchStep {
        OrchStep::SendPrimitive {
            action: "p:snapshot".to_string(),
            params: Value::Object(Map::new()),
        }
    }

    fn step(&mut self, response: Value) -> OrchStep {
        match self.phase {
            Phase::GetSnapshot => self.handle_snapshot(response),
            Phase::CreateWindow => self.handle_window_created(response),
            Phase::MoveRemaining => self.after_move(),
            Phase::MoveTabs => self.after_move(),
            Phase::GroupTabs => self.handle_grouped(response),
            Phase::UpdateGroup => self.verify_grouping(),
            Phase::Verify => self.handle_verify(response),
            Phase::RegroupStragglers => self.handle_regroup(response),
        }
    }
}

impl MoveGroupOrchestration {
    fn handle_snapshot(&mut self, snapshot: Value) -> OrchStep {
        let group_id_param = self.params.get("groupId").and_then(Value::as_i64);
        let group_title_param = self
            .params
            .get("groupTitle")
            .and_then(Value::as_str)
            .map(|s| s.trim())
            .filter(|s| !s.is_empty());

        let window_id_param = self
            .params
            .get("windowId")
            .and_then(|v| resolve_window_id(&snapshot, v));

        if group_id_param.is_none() && group_title_param.is_none() {
            return OrchStep::Error {
                message: "Missing group identifier".to_string(),
                hint: Some("Provide --group or --group-id.".to_string()),
            };
        }

        let gm = match resolve_group(
            &snapshot,
            group_id_param,
            group_title_param,
            window_id_param,
        ) {
            Ok(m) => m,
            Err(e) => return e,
        };

        let tab_ids: Vec<i64> = gm.tabs.iter().map(|t| t.tab_id).collect();
        if tab_ids.is_empty() {
            return OrchStep::Error {
                message: "Group has no tabs".to_string(),
                hint: None,
            };
        }

        let undo_tabs: Vec<Value> = gm
            .tabs
            .iter()
            .map(|t| {
                serde_json::json!({
                    "tabId": t.tab_id,
                    "windowId": t.window_id,
                    "index": t.index,
                    "groupId": t.group_id,
                    "groupTitle": t.group_title,
                    "groupColor": t.group_color,
                    "groupCollapsed": t.group_collapsed,
                })
            })
            .collect();

        let new_window = self
            .params
            .get("newWindow")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        let target_window_id = if new_window {
            None
        } else if let Some(raw) = self.params.get("targetWindowId") {
            match resolve_window_id(&snapshot, raw) {
                Some(wid) => Some(wid),
                None => {
                    return OrchStep::Error {
                        message: "Target window not found".to_string(),
                        hint: None,
                    }
                }
            }
        } else {
            Some(gm.window_id)
        };

        let is_cross_window = !matches!(target_window_id, Some(tw) if tw == gm.window_id);

        // Resolve positioning: afterGroupTitle, beforeGroupTitle, afterTabId, beforeTabId, index
        let has_positioning = self.params.get("afterGroupTitle").is_some()
            || self.params.get("beforeGroupTitle").is_some()
            || self.params.get("afterTabId").is_some()
            || self.params.get("beforeTabId").is_some();

        let target_index = if is_cross_window {
            -1
        } else if let Some(after_title) = self.params.get("afterGroupTitle").and_then(Value::as_str)
        {
            // Find anchor group's last tab index, move after it
            match resolve_group(&snapshot, None, Some(after_title), target_window_id) {
                Ok(anchor) => anchor
                    .tabs
                    .iter()
                    .filter_map(|t| t.index)
                    .max()
                    .map(|i| i + 1)
                    .unwrap_or(-1),
                Err(e) => return e,
            }
        } else if let Some(before_title) =
            self.params.get("beforeGroupTitle").and_then(Value::as_str)
        {
            match resolve_group(&snapshot, None, Some(before_title), target_window_id) {
                Ok(anchor) => anchor
                    .tabs
                    .iter()
                    .filter_map(|t| t.index)
                    .min()
                    .unwrap_or(-1),
                Err(e) => return e,
            }
        } else if let Some(after_tab) = self.params.get("afterTabId").and_then(Value::as_i64) {
            find_tab_index(&snapshot, after_tab)
                .map(|i| i + 1)
                .unwrap_or(-1)
        } else if let Some(before_tab) = self.params.get("beforeTabId").and_then(Value::as_i64) {
            find_tab_index(&snapshot, before_tab).unwrap_or(-1)
        } else {
            self.params
                .get("index")
                .and_then(Value::as_i64)
                .unwrap_or(-1)
        };

        // Chrome ungroups tabs when moved within the same window if they leave
        // their group's contiguous block, so re-group after any positioned move.
        let needs_regroup = is_cross_window || has_positioning;

        self.state = Some(MoveGroupState {
            source_group_id: gm.group_id,
            source_window_id: gm.window_id,
            has_incognito: gm.window_incognito
                || target_window_id
                    .map(|window_id| window_is_incognito(&snapshot, window_id))
                    .unwrap_or(false),
            group_title: gm.title.clone(),
            group_color: gm.color.clone(),
            group_collapsed: gm.collapsed,
            tab_ids: tab_ids.clone(),
            undo_tabs,
            target_window_id,
            new_group_id: None,
            needs_regroup,
            verify_attempts: 0,
        });

        if new_window {
            self.phase = Phase::CreateWindow;
            OrchStep::SendPrimitive {
                action: "p:window-create".to_string(),
                params: serde_json::json!({"tabId": tab_ids[0], "focused": false}),
            }
        } else {
            let tw = target_window_id.unwrap();
            self.phase = Phase::MoveTabs;
            OrchStep::SendPrimitive {
                action: "p:tab-move".to_string(),
                params: serde_json::json!({
                    "tabIds": tab_ids,
                    "windowId": tw,
                    "index": target_index,
                }),
            }
        }
    }

    fn handle_window_created(&mut self, response: Value) -> OrchStep {
        let window_id = response
            .get("id")
            .or_else(|| response.get("windowId"))
            .and_then(Value::as_i64);

        let Some(window_id) = window_id else {
            return OrchStep::Error {
                message: "Failed to create window".to_string(),
                hint: None,
            };
        };

        let state = self.state.as_mut().unwrap();
        state.target_window_id = Some(window_id);

        if state.tab_ids.len() > 1 {
            let remaining: Vec<i64> = state.tab_ids[1..].to_vec();
            self.phase = Phase::MoveRemaining;
            OrchStep::SendPrimitive {
                action: "p:tab-move".to_string(),
                params: serde_json::json!({
                    "tabIds": remaining,
                    "windowId": window_id,
                    "index": -1,
                }),
            }
        } else {
            self.send_group_step()
        }
    }

    fn after_move(&mut self) -> OrchStep {
        let state = self.state.as_ref().unwrap();
        if state.needs_regroup {
            self.send_group_step()
        } else {
            self.complete()
        }
    }

    fn send_group_step(&mut self) -> OrchStep {
        let state = self.state.as_ref().unwrap();
        let window_id = state.target_window_id.unwrap();

        self.phase = Phase::GroupTabs;
        OrchStep::SendPrimitive {
            action: "p:tab-group".to_string(),
            params: serde_json::json!({
                "tabIds": state.tab_ids,
                "createProperties": {"windowId": window_id},
            }),
        }
    }

    fn handle_grouped(&mut self, response: Value) -> OrchStep {
        let group_id = response
            .get("groupId")
            .and_then(Value::as_i64)
            .or_else(|| response.as_i64());

        let state = self.state.as_mut().unwrap();
        state.new_group_id = group_id;

        let mut update = Map::new();
        if let Some(title) = &state.group_title {
            update.insert("title".to_string(), Value::String(title.clone()));
        }
        if let Some(color) = &state.group_color {
            update.insert("color".to_string(), Value::String(color.clone()));
        }
        if let Some(collapsed) = state.group_collapsed {
            update.insert("collapsed".to_string(), Value::Bool(collapsed));
        }

        if let Some(gid) = group_id {
            if !update.is_empty() {
                self.phase = Phase::UpdateGroup;
                update.insert("groupId".to_string(), serde_json::json!(gid));
                return OrchStep::SendPrimitive {
                    action: "p:group-update".to_string(),
                    params: Value::Object(update),
                };
            }
        }

        self.complete()
    }

    fn verify_grouping(&mut self) -> OrchStep {
        let state = self.state.as_ref().unwrap();
        if let Some(window_id) = state.target_window_id {
            self.phase = Phase::Verify;
            OrchStep::SendPrimitive {
                action: "p:tab-query".to_string(),
                params: serde_json::json!({"query": {"windowId": window_id}}),
            }
        } else {
            self.complete()
        }
    }

    fn handle_verify(&mut self, response: Value) -> OrchStep {
        let state = self.state.as_mut().unwrap();
        let group_id = match state.new_group_id {
            Some(gid) => gid,
            None => return self.complete(),
        };

        state.verify_attempts += 1;

        let tabs = response.as_array();
        let straggler_ids: Vec<i64> = state
            .tab_ids
            .iter()
            .filter(|&&tid| {
                if let Some(tabs) = tabs {
                    tabs.iter().any(|t| {
                        t.get("id").and_then(Value::as_i64) == Some(tid)
                            && t.get("groupId").and_then(Value::as_i64) != Some(group_id)
                    })
                } else {
                    false
                }
            })
            .copied()
            .collect();

        if straggler_ids.is_empty() || state.verify_attempts > 3 {
            return self.complete();
        }

        self.phase = Phase::RegroupStragglers;
        OrchStep::SendPrimitive {
            action: "p:tab-group".to_string(),
            params: serde_json::json!({"groupId": group_id, "tabIds": straggler_ids}),
        }
    }

    fn handle_regroup(&mut self, _response: Value) -> OrchStep {
        // After regrouping stragglers, verify again
        self.verify_grouping()
    }

    fn complete(&self) -> OrchStep {
        let state = self.state.as_ref().unwrap();

        let mut response = serde_json::json!({
            "groupId": state.new_group_id.unwrap_or(state.source_group_id),
            "windowId": state.source_window_id,
            "movedToWindowId": state.target_window_id,
            "summary": {
                "movedTabs": state.tab_ids.len(),
            },
        });
        if let Some(new_gid) = state.new_group_id {
            response["newGroupId"] = serde_json::json!(new_gid);
        }

        OrchStep::Complete {
            response,
            undo: Some(serde_json::json!({
                "action": "move-group",
                "incognito": state.has_incognito,
                "groupId": state.source_group_id,
                "windowId": state.source_window_id,
                "movedToWindowId": state.target_window_id,
                "groupTitle": state.group_title,
                "groupColor": state.group_color,
                "groupCollapsed": state.group_collapsed,
                "tabs": state.undo_tabs,
            })),
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
                    {"tabId": 2, "windowId": 100, "index": 1, "groupId": 10, "groupTitle": "Dev", "groupColor": "blue", "groupCollapsed": false},
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
    fn move_group_cross_window() {
        let params = serde_json::json!({"groupId": 10, "targetWindowId": 200});
        let mut orch = MoveGroupOrchestration::new(&params);

        let step = orch.start();
        assert!(matches!(&step, OrchStep::SendPrimitive { action, .. } if action == "p:snapshot"));

        // Snapshot → tab-move to window 200
        let step = orch.step(snapshot());
        let OrchStep::SendPrimitive { action, params } = &step else {
            panic!("expected tab-move, got {step:?}");
        };
        assert_eq!(action, "p:tab-move");
        assert_eq!(params["windowId"], 200);
        assert_eq!(params["tabIds"], serde_json::json!([1, 2]));

        // Moved → tab-group (needs regroup for cross-window)
        let step = orch.step(serde_json::json!({}));
        let OrchStep::SendPrimitive { action, .. } = &step else {
            panic!("expected tab-group, got {step:?}");
        };
        assert_eq!(action, "p:tab-group");

        // Grouped → group-update with original properties
        let step = orch.step(serde_json::json!({"groupId": 50}));
        let OrchStep::SendPrimitive { action, params } = &step else {
            panic!("expected group-update, got {step:?}");
        };
        assert_eq!(action, "p:group-update");
        assert_eq!(params["title"], "Dev");
        assert_eq!(params["color"], "blue");

        // Updated → Verify (p:tab-query)
        let step = orch.step(serde_json::json!({}));
        let OrchStep::SendPrimitive { action, .. } = &step else {
            panic!("expected tab-query for verify, got {step:?}");
        };
        assert_eq!(action, "p:tab-query");

        // Verify → Complete (all tabs grouped, no stragglers)
        let step = orch.step(serde_json::json!([
            {"id": 1, "groupId": 50},
            {"id": 2, "groupId": 50}
        ]));
        let OrchStep::Complete { response, undo } = step else {
            panic!("expected Complete");
        };
        assert_eq!(response["summary"]["movedTabs"], 2);
        let undo = undo.unwrap();
        assert_eq!(undo["action"], "move-group");
        assert_eq!(undo["tabs"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn move_group_to_new_window() {
        let params = serde_json::json!({"groupId": 10, "newWindow": true});
        let mut orch = MoveGroupOrchestration::new(&params);
        let _ = orch.start();

        let step = orch.step(snapshot());
        let OrchStep::SendPrimitive { action, params } = &step else {
            panic!("expected window-create, got {step:?}");
        };
        assert_eq!(action, "p:window-create");
        assert_eq!(params["tabId"], 1);

        // Window created → move remaining tabs
        let step = orch.step(serde_json::json!({"id": 300}));
        let OrchStep::SendPrimitive { action, params } = &step else {
            panic!("expected tab-move, got {step:?}");
        };
        assert_eq!(action, "p:tab-move");
        assert_eq!(params["tabIds"], serde_json::json!([2]));
        assert_eq!(params["windowId"], 300);
    }

    #[test]
    fn move_group_same_window_no_regroup() {
        let params = serde_json::json!({"groupId": 10, "index": 2});
        let mut orch = MoveGroupOrchestration::new(&params);
        let _ = orch.start();

        // Snapshot → tab-move (same window, defaults to source)
        let step = orch.step(snapshot());
        let OrchStep::SendPrimitive { action, params } = &step else {
            panic!("expected tab-move, got {step:?}");
        };
        assert_eq!(action, "p:tab-move");
        assert_eq!(params["windowId"], 100);
        assert_eq!(params["index"], 2);

        // Moved → Complete directly (no re-grouping for same window)
        let step = orch.step(serde_json::json!({}));
        assert!(matches!(&step, OrchStep::Complete { .. }));
    }

    #[test]
    fn move_group_missing_identifier_errors() {
        let params = serde_json::json!({});
        let mut orch = MoveGroupOrchestration::new(&params);
        let _ = orch.start();
        let step = orch.step(serde_json::json!({"windows": []}));
        assert!(
            matches!(&step, OrchStep::Error { message, .. } if message.contains("Missing group"))
        );
    }

    #[test]
    fn move_group_after_group_title_regroups() {
        // Move group 10 ("Dev") after a second group in the same window.
        // Adds a second group to the snapshot.
        let snap = serde_json::json!({
            "windows": [{
                "windowId": 100,
                "focused": true,
                "tabs": [
                    {"tabId": 1, "windowId": 100, "index": 0, "groupId": 10, "groupTitle": "Dev", "groupColor": "blue", "groupCollapsed": false},
                    {"tabId": 2, "windowId": 100, "index": 1, "groupId": 10, "groupTitle": "Dev", "groupColor": "blue", "groupCollapsed": false},
                    {"tabId": 3, "windowId": 100, "index": 2, "groupId": -1},
                    {"tabId": 4, "windowId": 100, "index": 3, "groupId": 20, "groupTitle": "Anchor", "groupColor": "red", "groupCollapsed": false},
                    {"tabId": 5, "windowId": 100, "index": 4, "groupId": 20, "groupTitle": "Anchor", "groupColor": "red", "groupCollapsed": false},
                ],
                "groups": [
                    {"groupId": 10, "title": "Dev", "color": "blue", "collapsed": false},
                    {"groupId": 20, "title": "Anchor", "color": "red", "collapsed": false}
                ]
            }]
        });

        let params = serde_json::json!({"groupId": 10, "afterGroupTitle": "Anchor"});
        let mut orch = MoveGroupOrchestration::new(&params);
        let _ = orch.start();

        // Snapshot → tab-move with index 5 (after Anchor's last tab at index 4)
        let step = orch.step(snap);
        let OrchStep::SendPrimitive { action, params } = &step else {
            panic!("expected tab-move, got {step:?}");
        };
        assert_eq!(action, "p:tab-move");
        assert_eq!(params["windowId"], 100);
        assert_eq!(params["index"], 5);
        assert_eq!(params["tabIds"], serde_json::json!([1, 2]));

        // After move → tab-group (needs_regroup is true for positioned moves)
        let step = orch.step(serde_json::json!({}));
        let OrchStep::SendPrimitive { action, .. } = &step else {
            panic!("expected tab-group, got {step:?}");
        };
        assert_eq!(action, "p:tab-group");

        // After group → group-update
        let step = orch.step(serde_json::json!({"groupId": 50}));
        let OrchStep::SendPrimitive { action, params } = &step else {
            panic!("expected group-update, got {step:?}");
        };
        assert_eq!(action, "p:group-update");
        assert_eq!(params["title"], "Dev");
        assert_eq!(params["color"], "blue");

        // After update → verify
        let step = orch.step(serde_json::json!({}));
        assert!(matches!(&step, OrchStep::SendPrimitive { action, .. } if action == "p:tab-query"));

        // Verify → Complete
        let step = orch.step(serde_json::json!([
            {"id": 1, "groupId": 50},
            {"id": 2, "groupId": 50}
        ]));
        assert!(matches!(&step, OrchStep::Complete { .. }));
    }
}
