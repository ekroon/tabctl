use serde_json::{Map, Value};

use super::resolve::{resolve_group, resolve_window_id};
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

/// Orchestration for the `move-tab` command.
///
/// p:snapshot → resolve source tab + target → p:tab-move or p:window-create
/// → Complete with undo.
#[derive(Debug)]
pub(crate) struct MoveTabOrchestration {
    params: Value,
    phase: Phase,
    state: Option<MoveState>,
}

#[derive(Debug)]
struct MoveState {
    tab_id: i64,
    from_window_id: i64,
    from_index: Option<i64>,
    from_group_id: i64,
    from_group_title: Option<String>,
    from_group_color: Option<String>,
    from_group_collapsed: Option<bool>,
    to_window_id: Option<i64>,
    to_index: Option<i64>,
}

#[derive(Debug)]
enum Phase {
    GetSnapshot,
    MoveTab,
    WindowCreated,
}

impl MoveTabOrchestration {
    pub(crate) fn new(params: &Value) -> Self {
        Self {
            params: params.clone(),
            phase: Phase::GetSnapshot,
            state: None,
        }
    }
}

impl super::Orchestration for MoveTabOrchestration {
    fn start(&mut self) -> OrchStep {
        OrchStep::SendPrimitive {
            action: "p:snapshot".to_string(),
            params: Value::Object(Map::new()),
        }
    }

    fn step(&mut self, response: Value) -> OrchStep {
        match self.phase {
            Phase::GetSnapshot => self.handle_snapshot(response),
            Phase::MoveTab => self.complete(),
            Phase::WindowCreated => self.handle_window_created(response),
        }
    }
}

impl MoveTabOrchestration {
    fn handle_snapshot(&mut self, snapshot: Value) -> OrchStep {
        let tab_id = match self.params.get("tabId").and_then(Value::as_i64) {
            Some(id) => id,
            None => match self
                .params
                .get("tabIds")
                .and_then(Value::as_array)
                .and_then(|a| a.first())
                .and_then(Value::as_i64)
            {
                Some(id) => id,
                None => {
                    return OrchStep::Error {
                        message: "Missing tabId".to_string(),
                        hint: None,
                    }
                }
            },
        };

        // Find the source tab in the snapshot
        let all_tabs =
            super::scope::select_tabs_by_scope(&snapshot, &serde_json::json!({"all": true})).tabs;
        let source_tab = match all_tabs.iter().find(|t| t.tab_id == tab_id) {
            Some(t) => t,
            None => {
                return OrchStep::Error {
                    message: format!("Tab {tab_id} not found"),
                    hint: None,
                }
            }
        };

        let group_index = build_group_index(&snapshot);
        let (group_title, group_color, group_collapsed) = if source_tab.group_id != -1 {
            group_index
                .get(&source_tab.group_id)
                .map(|g| (g.0.clone(), g.1.clone(), g.2))
                .unwrap_or((
                    source_tab.group_title.clone(),
                    source_tab.group_color.clone(),
                    source_tab.group_collapsed,
                ))
        } else {
            (None, None, None)
        };

        let new_window = self
            .params
            .get("newWindow")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        self.state = Some(MoveState {
            tab_id,
            from_window_id: source_tab.window_id,
            from_index: source_tab.index,
            from_group_id: source_tab.group_id,
            from_group_title: group_title,
            from_group_color: group_color,
            from_group_collapsed: group_collapsed,
            to_window_id: None,
            to_index: None,
        });

        if new_window {
            self.phase = Phase::WindowCreated;
            OrchStep::SendPrimitive {
                action: "p:window-create".to_string(),
                params: serde_json::json!({"tabId": tab_id}),
            }
        } else {
            let target_window_id = self
                .params
                .get("windowId")
                .and_then(|v| resolve_window_id(&snapshot, v))
                .unwrap_or(source_tab.window_id);

            let target_index = if let Some(after_title) =
                self.params.get("afterGroupTitle").and_then(Value::as_str)
            {
                match resolve_group(&snapshot, None, Some(after_title), Some(target_window_id)) {
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
                match resolve_group(&snapshot, None, Some(before_title), Some(target_window_id)) {
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
            } else if let Some(before_tab) = self.params.get("beforeTabId").and_then(Value::as_i64)
            {
                find_tab_index(&snapshot, before_tab).unwrap_or(-1)
            } else {
                self.params
                    .get("index")
                    .and_then(Value::as_i64)
                    .unwrap_or(-1)
            };

            if let Some(state) = self.state.as_mut() {
                state.to_window_id = Some(target_window_id);
                state.to_index = Some(target_index);
            }

            self.phase = Phase::MoveTab;
            OrchStep::SendPrimitive {
                action: "p:tab-move".to_string(),
                params: serde_json::json!({
                    "tabIds": [tab_id],
                    "windowId": target_window_id,
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

        if let Some(state) = self.state.as_mut() {
            state.to_window_id = window_id;
            state.to_index = Some(0);
        }

        self.complete()
    }

    fn complete(&self) -> OrchStep {
        let state = self.state.as_ref().unwrap();

        OrchStep::Complete {
            response: serde_json::json!({
                "tabId": state.tab_id,
                "windowId": state.to_window_id,
                "summary": {
                    "movedTabs": 1,
                },
            }),
            undo: Some(serde_json::json!({
                "action": "move-tab",
                "tabId": state.tab_id,
                "from": {
                    "windowId": state.from_window_id,
                    "index": state.from_index,
                    "groupId": state.from_group_id,
                    "groupTitle": state.from_group_title,
                    "groupColor": state.from_group_color,
                    "groupCollapsed": state.from_group_collapsed,
                },
                "to": {
                    "windowId": state.to_window_id,
                    "index": state.to_index,
                },
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
    fn move_tab_same_window() {
        let params = serde_json::json!({"tabId": 1, "windowId": 100, "index": 1});
        let mut orch = MoveTabOrchestration::new(&params);

        let step = orch.start();
        assert!(matches!(&step, OrchStep::SendPrimitive { action, .. } if action == "p:snapshot"));

        let step = orch.step(snapshot());
        let OrchStep::SendPrimitive { action, params } = &step else {
            panic!("expected tab-move, got {step:?}");
        };
        assert_eq!(action, "p:tab-move");
        assert_eq!(params["tabIds"], serde_json::json!([1]));
        assert_eq!(params["windowId"], 100);

        let step = orch.step(serde_json::json!({}));
        let OrchStep::Complete { response, undo } = step else {
            panic!("expected Complete");
        };
        assert_eq!(response["summary"]["movedTabs"], 1);
        let undo = undo.unwrap();
        assert_eq!(undo["action"], "move-tab");
        assert_eq!(undo["from"]["windowId"], 100);
        assert_eq!(undo["from"]["groupTitle"], "Dev");
    }

    #[test]
    fn move_tab_cross_window() {
        let params = serde_json::json!({"tabId": 1, "windowId": 200});
        let mut orch = MoveTabOrchestration::new(&params);
        let _ = orch.start();

        let step = orch.step(snapshot());
        let OrchStep::SendPrimitive { action, params } = &step else {
            panic!("expected tab-move, got {step:?}");
        };
        assert_eq!(action, "p:tab-move");
        assert_eq!(params["windowId"], 200);
    }

    #[test]
    fn move_tab_to_new_window() {
        let params = serde_json::json!({"tabId": 2, "newWindow": true});
        let mut orch = MoveTabOrchestration::new(&params);
        let _ = orch.start();

        let step = orch.step(snapshot());
        let OrchStep::SendPrimitive { action, params } = &step else {
            panic!("expected window-create, got {step:?}");
        };
        assert_eq!(action, "p:window-create");
        assert_eq!(params["tabId"], 2);

        let step = orch.step(serde_json::json!({"id": 300}));
        let OrchStep::Complete { undo, .. } = step else {
            panic!("expected Complete");
        };
        let undo = undo.unwrap();
        assert_eq!(undo["to"]["windowId"], 300);
    }

    #[test]
    fn move_tab_missing_id_errors() {
        let params = serde_json::json!({});
        let mut orch = MoveTabOrchestration::new(&params);
        let _ = orch.start();
        let step = orch.step(serde_json::json!({"windows": []}));
        assert!(
            matches!(&step, OrchStep::Error { message, .. } if message.contains("Missing tabId"))
        );
    }
}
