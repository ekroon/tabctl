use serde_json::{Map, Value};

use super::resolve::{resolve_group, resolve_window_id};
use super::OrchStep;

/// Orchestration for the `group-ungroup` command.
///
/// Two steps: p:snapshot → resolve group + capture tabs for undo → p:tab-ungroup.
#[derive(Debug)]
pub(crate) struct GroupUngroupOrchestration {
    params: Value,
    phase: GroupUngroupPhase,
    pre_state: Option<UngroupPreState>,
}

#[derive(Debug)]
struct UngroupPreState {
    group_id: i64,
    window_id: i64,
    group_title: Option<String>,
    group_color: Option<String>,
    group_collapsed: Option<bool>,
    tab_ids: Vec<i64>,
    undo_tabs: Vec<Value>,
}

#[derive(Debug)]
enum GroupUngroupPhase {
    GetSnapshot,
    Ungroup,
}

impl GroupUngroupOrchestration {
    pub(crate) fn new(params: &Value) -> Self {
        Self {
            params: params.clone(),
            phase: GroupUngroupPhase::GetSnapshot,
            pre_state: None,
        }
    }
}

impl super::Orchestration for GroupUngroupOrchestration {
    fn start(&mut self) -> OrchStep {
        OrchStep::SendPrimitive {
            action: "p:snapshot".to_string(),
            params: Value::Object(Map::new()),
        }
    }

    fn step(&mut self, response: Value) -> OrchStep {
        match self.phase {
            GroupUngroupPhase::GetSnapshot => {
                let group_id = self.params.get("groupId").and_then(Value::as_i64);
                let group_title = self.params.get("groupTitle").and_then(Value::as_str);

                if group_id.is_none() && group_title.map_or(true, |s| s.trim().is_empty()) {
                    return OrchStep::Error {
                        message: "Missing group identifier".to_string(),
                        hint: None,
                    };
                }

                let window_id_param = self
                    .params
                    .get("windowId")
                    .and_then(|v| resolve_window_id(&response, v));

                let matched = match resolve_group(&response, group_id, group_title, window_id_param)
                {
                    Ok(m) => m,
                    Err(e) => return e,
                };

                let tab_ids: Vec<i64> = matched.tabs.iter().map(|t| t.tab_id).collect();
                if tab_ids.is_empty() {
                    return OrchStep::Complete {
                        response: serde_json::json!({
                            "groupId": matched.group_id,
                            "windowId": matched.window_id,
                            "summary": { "ungroupedTabs": 0 },
                        }),
                        undo: None,
                    };
                }

                let undo_tabs: Vec<Value> = matched
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

                self.pre_state = Some(UngroupPreState {
                    group_id: matched.group_id,
                    window_id: matched.window_id,
                    group_title: matched.title,
                    group_color: matched.color,
                    group_collapsed: matched.collapsed,
                    tab_ids: tab_ids.clone(),
                    undo_tabs,
                });

                self.phase = GroupUngroupPhase::Ungroup;
                OrchStep::SendPrimitive {
                    action: "p:tab-ungroup".to_string(),
                    params: serde_json::json!({ "tabIds": tab_ids }),
                }
            }
            GroupUngroupPhase::Ungroup => {
                let pre = self.pre_state.as_ref().unwrap();

                let undo = serde_json::json!({
                    "action": "group-ungroup",
                    "groupId": pre.group_id,
                    "windowId": pre.window_id,
                    "groupTitle": pre.group_title,
                    "groupColor": pre.group_color,
                    "groupCollapsed": pre.group_collapsed,
                    "tabs": pre.undo_tabs,
                });

                OrchStep::Complete {
                    response: serde_json::json!({
                        "groupId": pre.group_id,
                        "windowId": pre.window_id,
                        "summary": { "ungroupedTabs": pre.tab_ids.len() },
                    }),
                    undo: Some(undo),
                }
            }
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
            }]
        })
    }

    #[test]
    fn ungroup_by_id_with_undo() {
        let params = serde_json::json!({"groupId": 10});
        let mut orch = GroupUngroupOrchestration::new(&params);

        let step = orch.start();
        assert!(matches!(&step, OrchStep::SendPrimitive { action, .. } if action == "p:snapshot"));

        let step = orch.step(snapshot());
        let OrchStep::SendPrimitive { action, params } = &step else {
            panic!("expected SendPrimitive, got {step:?}");
        };
        assert_eq!(action, "p:tab-ungroup");
        let tab_ids = params["tabIds"].as_array().unwrap();
        assert_eq!(tab_ids.len(), 2);

        let step = orch.step(serde_json::json!({"ungrouped": true}));
        let OrchStep::Complete { response, undo } = step else {
            panic!("expected Complete");
        };
        assert_eq!(response["summary"]["ungroupedTabs"], 2);

        let undo = undo.unwrap();
        assert_eq!(undo["action"], "group-ungroup");
        assert_eq!(undo["groupTitle"], "Dev");
        assert_eq!(undo["groupColor"], "blue");
        let undo_tabs = undo["tabs"].as_array().unwrap();
        assert_eq!(undo_tabs.len(), 2);
        assert_eq!(undo_tabs[0]["tabId"], 1);
    }

    #[test]
    fn ungroup_by_title() {
        let params = serde_json::json!({"groupTitle": "Dev"});
        let mut orch = GroupUngroupOrchestration::new(&params);
        let _ = orch.start();

        let step = orch.step(snapshot());
        assert!(
            matches!(&step, OrchStep::SendPrimitive { action, .. } if action == "p:tab-ungroup")
        );
    }

    #[test]
    fn ungroup_missing_identifier_errors() {
        let params = serde_json::json!({});
        let mut orch = GroupUngroupOrchestration::new(&params);
        let _ = orch.start();
        let step = orch.step(snapshot());
        assert!(matches!(&step, OrchStep::Error { .. }));
    }
}
