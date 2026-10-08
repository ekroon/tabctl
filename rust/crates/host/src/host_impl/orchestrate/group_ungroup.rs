use serde_json::{Map, Value};

use super::resolve::{resolve_group, resolve_window_id};
use super::scope::select_tabs_by_scope;
use super::OrchStep;

/// Orchestration for the `group-ungroup` command.
///
/// Resolve explicit tabs or a group; recovery is captured by the host boundary.
#[derive(Debug)]
pub(crate) struct GroupUngroupOrchestration {
    params: Value,
    phase: GroupUngroupPhase,
    pre_state: Option<UngroupPreState>,
}

#[derive(Debug)]
struct UngroupPreState {
    group_id: Option<i64>,
    window_id: Option<i64>,
    tab_ids: Vec<i64>,
    skipped: Vec<Value>,
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
                let mut skipped = Vec::new();
                let (group_id, window_id, tab_ids) = if self.params.get("tabIds").is_some() {
                    let scope = select_tabs_by_scope(&response, &self.params);
                    if let Some(message) = scope.error {
                        return OrchStep::Error {
                            message,
                            hint: None,
                        };
                    }
                    for id in self.params["tabIds"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_i64)
                    {
                        if !scope.tabs.iter().any(|tab| tab.tab_id == id) {
                            skipped.push(serde_json::json!({"tabId":id,"reason":"not_found"}));
                        }
                    }
                    let tabs: Vec<_> = scope.tabs.into_iter().filter(|tab| {
                        if tab.group_id == -1 {
                            skipped.push(serde_json::json!({"tabId":tab.tab_id,"reason":"already_ungrouped"}));
                            false
                        } else { true }
                    }).collect();
                    let group_id = tabs
                        .first()
                        .map(|tab| tab.group_id)
                        .filter(|id| tabs.iter().all(|tab| tab.group_id == *id));
                    let window_id = tabs
                        .first()
                        .map(|tab| tab.window_id)
                        .filter(|id| tabs.iter().all(|tab| tab.window_id == *id));
                    (
                        group_id,
                        window_id,
                        tabs.iter().map(|tab| tab.tab_id).collect::<Vec<_>>(),
                    )
                } else {
                    let group_id = self.params.get("groupId").and_then(Value::as_i64);
                    let group_title = self.params.get("groupTitle").and_then(Value::as_str);
                    if group_id.is_none()
                        && group_title.map_or(true, |title| title.trim().is_empty())
                    {
                        return OrchStep::Error {
                            message: "Missing group identifier".into(),
                            hint: None,
                        };
                    }
                    let window_id = self
                        .params
                        .get("windowId")
                        .and_then(|id| resolve_window_id(&response, id));
                    let matched = match resolve_group(&response, group_id, group_title, window_id) {
                        Ok(matched) => matched,
                        Err(error) => return error,
                    };
                    (
                        Some(matched.group_id),
                        Some(matched.window_id),
                        matched.tabs.iter().map(|tab| tab.tab_id).collect(),
                    )
                };
                if tab_ids.is_empty() {
                    return OrchStep::Complete {
                        response: serde_json::json!({
                            "groupId": group_id,
                            "windowId": window_id,
                            "summary": { "ungroupedTabs": 0, "skippedTabs": skipped.len() },
                            "skipped": skipped,
                        }),
                        undo: None,
                    };
                }

                self.pre_state = Some(UngroupPreState {
                    group_id,
                    window_id,
                    tab_ids: tab_ids.clone(),
                    skipped,
                });

                self.phase = GroupUngroupPhase::Ungroup;
                OrchStep::SendPrimitive {
                    action: "p:tab-ungroup".to_string(),
                    params: serde_json::json!({ "tabIds": tab_ids }),
                }
            }
            GroupUngroupPhase::Ungroup => {
                let pre = self.pre_state.as_ref().unwrap();

                OrchStep::Complete {
                    response: serde_json::json!({
                        "groupId": pre.group_id,
                        "windowId": pre.window_id,
                        "summary": { "ungroupedTabs": pre.tab_ids.len(), "skippedTabs": pre.skipped.len() },
                        "skipped": pre.skipped,
                    }),
                    undo: None,
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
    fn ungroup_by_id() {
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

        assert!(undo.is_none());
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

    #[test]
    fn explicit_tabs_never_ungroup_other_group_members() {
        let mut orch = GroupUngroupOrchestration::new(&serde_json::json!({"tabIds":[2]}));
        let _ = orch.start();
        let OrchStep::SendPrimitive { action, params } = orch.step(snapshot()) else {
            panic!("expected exact ungroup primitive")
        };
        assert_eq!(action, "p:tab-ungroup");
        assert_eq!(params["tabIds"], serde_json::json!([2]));
        for ids in [serde_json::json!([]), serde_json::json!([3])] {
            let mut orch = GroupUngroupOrchestration::new(&serde_json::json!({"tabIds":ids}));
            let _ = orch.start();
            let OrchStep::Complete { response, .. } = orch.step(snapshot()) else {
                panic!("empty or already-ungrouped selection must not mutate")
            };
            assert_eq!(response["summary"]["ungroupedTabs"], 0);
        }
    }
}
