use serde_json::Value;

use super::OrchStep;

/// Orchestration for the `refresh` command.
///
/// Reloads each tab ID sequentially. Fixes the existing single-tab bug by
/// iterating over ALL provided tab IDs instead of only the first.
#[derive(Debug)]
pub(crate) struct RefreshOrchestration {
    tab_ids: Vec<i64>,
    current_idx: usize,
}

impl RefreshOrchestration {
    pub(crate) fn new(params: &Value) -> Result<Self, OrchStep> {
        let mut tab_ids: Vec<i64> = params
            .get("tabIds")
            .and_then(Value::as_array)
            .map(|arr| arr.iter().filter_map(Value::as_i64).collect())
            .unwrap_or_default();

        if let Some(single) = params.get("tabId").and_then(Value::as_i64) {
            if !tab_ids.contains(&single) {
                tab_ids.insert(0, single);
            }
        }

        if tab_ids.is_empty() {
            return Err(OrchStep::Error {
                message: "Missing tabId or tabIds".to_string(),
                hint: None,
            });
        }

        Ok(Self {
            tab_ids,
            current_idx: 0,
        })
    }
}

impl super::Orchestration for RefreshOrchestration {
    fn start(&mut self) -> OrchStep {
        OrchStep::SendPrimitive {
            action: "p:tab-reload".to_string(),
            params: serde_json::json!({ "tabId": self.tab_ids[0] }),
        }
    }

    fn step(&mut self, _response: Value) -> OrchStep {
        self.current_idx += 1;
        if self.current_idx < self.tab_ids.len() {
            OrchStep::SendPrimitive {
                action: "p:tab-reload".to_string(),
                params: serde_json::json!({ "tabId": self.tab_ids[self.current_idx] }),
            }
        } else {
            OrchStep::Complete {
                response: serde_json::json!({
                    "summary": { "refreshedTabs": self.tab_ids.len() }
                }),
                undo: None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host_impl::orchestrate::Orchestration;

    #[test]
    fn refresh_single_tab() {
        let params = serde_json::json!({"tabId": 1});
        let mut orch = RefreshOrchestration::new(&params).unwrap();

        let step = orch.start();
        assert!(matches!(&step, OrchStep::SendPrimitive { action, params }
                if action == "p:tab-reload" && params["tabId"] == 1));

        let step = orch.step(serde_json::json!({"reloaded": true}));
        let OrchStep::Complete { response, .. } = step else {
            panic!("expected Complete");
        };
        assert_eq!(response["summary"]["refreshedTabs"], 1);
    }

    #[test]
    fn refresh_multiple_tabs_fixes_bug() {
        let params = serde_json::json!({"tabIds": [1, 2, 3]});
        let mut orch = RefreshOrchestration::new(&params).unwrap();

        let step = orch.start();
        assert!(matches!(&step, OrchStep::SendPrimitive { params, .. } if params["tabId"] == 1));

        let step = orch.step(serde_json::json!({"reloaded": true}));
        assert!(matches!(&step, OrchStep::SendPrimitive { params, .. } if params["tabId"] == 2));

        let step = orch.step(serde_json::json!({"reloaded": true}));
        assert!(matches!(&step, OrchStep::SendPrimitive { params, .. } if params["tabId"] == 3));

        let step = orch.step(serde_json::json!({"reloaded": true}));
        let OrchStep::Complete { response, .. } = step else {
            panic!("expected Complete");
        };
        assert_eq!(response["summary"]["refreshedTabs"], 3);
    }

    #[test]
    fn refresh_missing_tab_id_errors() {
        let params = serde_json::json!({});
        let r = RefreshOrchestration::new(&params);
        assert!(r.is_err());
    }

    #[test]
    fn refresh_tab_id_and_tab_ids_merge() {
        let params = serde_json::json!({"tabId": 5, "tabIds": [5, 6]});
        let orch = RefreshOrchestration::new(&params).unwrap();
        assert_eq!(orch.tab_ids, vec![5, 6]);
    }
}
