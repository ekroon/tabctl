use serde_json::Value;

use super::OrchStep;

/// Orchestration for the `focus` command.
///
/// Three steps: p:tab-get → p:window-update(focused) → p:tab-update(active).
#[derive(Debug)]
pub(crate) struct FocusOrchestration {
    tab_id: i64,
    window_id: Option<i64>,
    phase: FocusPhase,
}

#[derive(Debug)]
enum FocusPhase {
    GetTab,
    FocusWindow,
    ActivateTab,
}

impl FocusOrchestration {
    pub(crate) fn new(params: &Value) -> Result<Self, OrchStep> {
        let tab_ids = params.get("tabIds").and_then(Value::as_array);
        let tab_id = params
            .get("tabId")
            .and_then(Value::as_i64)
            .or_else(|| tab_ids.and_then(|ids| ids.first()).and_then(Value::as_i64));

        let Some(tab_id) = tab_id else {
            return Err(OrchStep::Error {
                message: "Missing tabId".to_string(),
                hint: None,
            });
        };

        Ok(Self {
            tab_id,
            window_id: None,
            phase: FocusPhase::GetTab,
        })
    }
}

impl super::Orchestration for FocusOrchestration {
    fn start(&mut self) -> OrchStep {
        OrchStep::SendPrimitive {
            action: "p:tab-get".to_string(),
            params: serde_json::json!({ "tabId": self.tab_id }),
        }
    }

    fn step(&mut self, response: Value) -> OrchStep {
        match self.phase {
            FocusPhase::GetTab => {
                self.window_id = response.get("windowId").and_then(Value::as_i64);
                let Some(window_id) = self.window_id else {
                    return OrchStep::Error {
                        message: "Tab has no windowId".to_string(),
                        hint: None,
                    };
                };
                self.phase = FocusPhase::FocusWindow;
                OrchStep::SendPrimitive {
                    action: "p:window-update".to_string(),
                    params: serde_json::json!({
                        "windowId": window_id,
                        "updateProperties": { "focused": true }
                    }),
                }
            }
            FocusPhase::FocusWindow => {
                self.phase = FocusPhase::ActivateTab;
                OrchStep::SendPrimitive {
                    action: "p:tab-update".to_string(),
                    params: serde_json::json!({
                        "tabId": self.tab_id,
                        "updateProperties": { "active": true }
                    }),
                }
            }
            FocusPhase::ActivateTab => OrchStep::Complete {
                response: serde_json::json!({
                    "tabId": self.tab_id,
                    "windowId": self.window_id,
                }),
                undo: None,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host_impl::orchestrate::Orchestration;

    #[test]
    fn focus_three_step_sequence() {
        let params = serde_json::json!({"tabId": 42});
        let mut orch = FocusOrchestration::new(&params).unwrap();

        // Step 1: request tab-get
        let step = orch.start();
        assert!(matches!(&step, OrchStep::SendPrimitive { action, params }
                if action == "p:tab-get" && params["tabId"] == 42));

        // Step 2: got tab → request window-update
        let step = orch.step(serde_json::json!({"id": 42, "windowId": 100}));
        assert!(matches!(&step, OrchStep::SendPrimitive { action, params }
                if action == "p:window-update" && params["windowId"] == 100));

        // Step 3: window focused → request tab-update
        let step = orch.step(serde_json::json!({"id": 100}));
        assert!(matches!(&step, OrchStep::SendPrimitive { action, params }
                if action == "p:tab-update" && params["tabId"] == 42));

        // Step 4: tab activated → complete
        let step = orch.step(serde_json::json!({"id": 42}));
        let OrchStep::Complete { response, undo } = step else {
            panic!("expected Complete");
        };
        assert_eq!(response["tabId"], 42);
        assert_eq!(response["windowId"], 100);
        assert!(undo.is_none());
    }

    #[test]
    fn focus_accepts_tab_ids_array() {
        let params = serde_json::json!({"tabIds": [7, 8, 9]});
        let mut orch = FocusOrchestration::new(&params).unwrap();
        let step = orch.start();
        assert!(matches!(&step, OrchStep::SendPrimitive { params, .. }
                if params["tabId"] == 7));
    }

    #[test]
    fn focus_missing_tab_id_errors() {
        let params = serde_json::json!({});
        let r = FocusOrchestration::new(&params);
        assert!(r.is_err());
    }
}
