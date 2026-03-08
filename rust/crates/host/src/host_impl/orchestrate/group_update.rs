use serde_json::{Map, Value};

use super::resolve::{resolve_group, resolve_window_id};
use super::OrchStep;

/// Orchestration for the `group-update` command.
///
/// Two steps: p:snapshot → resolve group + capture pre-state → p:group-update.
/// Produces an undo record with previous title/color/collapsed values.
#[derive(Debug)]
pub(crate) struct GroupUpdateOrchestration {
    params: Value,
    phase: GroupUpdatePhase,
    // Captured from snapshot for undo record
    pre_state: Option<PreState>,
}

#[derive(Debug)]
struct PreState {
    group_id: i64,
    window_id: i64,
    incognito: bool,
    title: Option<String>,
    color: Option<String>,
    collapsed: Option<bool>,
}

#[derive(Debug)]
enum GroupUpdatePhase {
    GetSnapshot,
    UpdateGroup,
}

impl GroupUpdateOrchestration {
    pub(crate) fn new(params: &Value) -> Self {
        Self {
            params: params.clone(),
            phase: GroupUpdatePhase::GetSnapshot,
            pre_state: None,
        }
    }
}

impl super::Orchestration for GroupUpdateOrchestration {
    fn start(&mut self) -> OrchStep {
        OrchStep::SendPrimitive {
            action: "p:snapshot".to_string(),
            params: Value::Object(Map::new()),
        }
    }

    fn step(&mut self, response: Value) -> OrchStep {
        match self.phase {
            GroupUpdatePhase::GetSnapshot => {
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

                // Build update properties
                let mut update = Map::new();
                if let Some(title) = self.params.get("title").and_then(Value::as_str) {
                    update.insert("title".to_string(), Value::String(title.to_string()));
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

                if update.is_empty() {
                    return OrchStep::Error {
                        message: "Missing group update fields".to_string(),
                        hint: None,
                    };
                }

                self.pre_state = Some(PreState {
                    group_id: matched.group_id,
                    window_id: matched.window_id,
                    incognito: matched.window_incognito,
                    title: matched.title,
                    color: matched.color,
                    collapsed: matched.collapsed,
                });

                self.phase = GroupUpdatePhase::UpdateGroup;
                update.insert("groupId".to_string(), serde_json::json!(matched.group_id));
                OrchStep::SendPrimitive {
                    action: "p:group-update".to_string(),
                    params: Value::Object(update),
                }
            }
            GroupUpdatePhase::UpdateGroup => {
                let pre = self.pre_state.as_ref().unwrap();

                let undo = serde_json::json!({
                    "action": "group-update",
                    "incognito": pre.incognito,
                    "groupId": pre.group_id,
                    "windowId": pre.window_id,
                    "previous": {
                        "title": pre.title,
                        "color": pre.color,
                        "collapsed": pre.collapsed,
                    }
                });

                OrchStep::Complete {
                    response: serde_json::json!({
                        "groupId": pre.group_id,
                        "windowId": pre.window_id,
                        "summary": { "updatedGroups": 1 },
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
                    {"tabId": 1, "windowId": 100, "index": 0, "groupId": 10},
                    {"tabId": 2, "windowId": 100, "index": 1, "groupId": 10}
                ],
                "groups": [
                    {"groupId": 10, "title": "Work", "color": "blue", "collapsed": false}
                ]
            }]
        })
    }

    #[test]
    fn group_update_by_id_with_undo() {
        let params = serde_json::json!({
            "groupId": 10,
            "title": "NewTitle",
            "color": "red"
        });
        let mut orch = GroupUpdateOrchestration::new(&params);

        // Step 1: request snapshot
        let step = orch.start();
        assert!(matches!(&step, OrchStep::SendPrimitive { action, .. } if action == "p:snapshot"));

        // Step 2: got snapshot → request group-update
        let step = orch.step(snapshot());
        let OrchStep::SendPrimitive { action, params } = &step else {
            panic!("expected SendPrimitive, got {step:?}");
        };
        assert_eq!(action, "p:group-update");
        assert_eq!(params["groupId"], 10);
        assert_eq!(params["title"], "NewTitle");
        assert_eq!(params["color"], "red");

        // Step 3: group updated → complete with undo
        let step = orch.step(serde_json::json!({"id": 10, "title": "NewTitle", "color": "red"}));
        let OrchStep::Complete { response, undo } = step else {
            panic!("expected Complete");
        };
        assert_eq!(response["groupId"], 10);
        assert_eq!(response["summary"]["updatedGroups"], 1);

        let undo = undo.unwrap();
        assert_eq!(undo["action"], "group-update");
        assert_eq!(undo["previous"]["title"], "Work");
        assert_eq!(undo["previous"]["color"], "blue");
        assert_eq!(undo["previous"]["collapsed"], false);
    }

    #[test]
    fn group_update_by_title() {
        let params = serde_json::json!({"groupTitle": "Work", "collapsed": true});
        let mut orch = GroupUpdateOrchestration::new(&params);
        let _ = orch.start();
        let step = orch.step(snapshot());
        assert!(matches!(&step, OrchStep::SendPrimitive { action, params }
            if action == "p:group-update" && params["collapsed"] == true));
    }

    #[test]
    fn group_update_missing_identifier_errors() {
        let params = serde_json::json!({"title": "X"});
        let mut orch = GroupUpdateOrchestration::new(&params);
        let _ = orch.start();
        let step = orch.step(snapshot());
        assert!(
            matches!(&step, OrchStep::Error { message, .. } if message.contains("Missing group identifier"))
        );
    }

    #[test]
    fn group_update_missing_fields_errors() {
        let params = serde_json::json!({"groupId": 10});
        let mut orch = GroupUpdateOrchestration::new(&params);
        let _ = orch.start();
        let step = orch.step(snapshot());
        assert!(
            matches!(&step, OrchStep::Error { message, .. } if message.contains("Missing group update fields"))
        );
    }
}
