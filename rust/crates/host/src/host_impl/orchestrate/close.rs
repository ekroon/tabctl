use serde_json::{json, Value};
use std::sync::Arc;

use super::scope::{select_tabs_by_scope, ScopedTab};
use super::{OrchStep, Orchestration};
use crate::host_impl::policy::Policy;

#[derive(Debug)]
pub(crate) struct CloseOrchestration {
    params: Value,
    policy: Arc<Policy>,
    plan: Option<Plan>,
}

#[derive(Debug)]
struct Plan {
    tabs: Vec<ScopedTab>,
    skipped: Vec<Value>,
}

impl CloseOrchestration {
    pub(crate) fn new(params: &Value) -> Self {
        Self {
            params: params.clone(),
            policy: Arc::new(Policy::default()),
            plan: None,
        }
    }

    fn preview(&self) -> bool {
        self.params
            .get("confirmed")
            .or_else(|| self.params.get("confirm"))
            .and_then(Value::as_bool)
            != Some(true)
            || self.params["dryRun"].as_bool() == Some(true)
    }
}

impl Orchestration for CloseOrchestration {
    fn set_policy(&mut self, policy: Arc<Policy>) {
        self.policy = policy;
    }

    fn start(&mut self) -> OrchStep {
        OrchStep::SendPrimitive {
            action: "p:snapshot".into(),
            params: json!({}),
        }
    }

    fn step(&mut self, response: Value) -> OrchStep {
        if let Some(plan) = self.plan.as_ref() {
            return OrchStep::Complete {
                response: json!({
                    "dryRun": false,
                    "summary": {"closedTabs":plan.tabs.len(),"skippedTabs":plan.skipped.len()},
                    "skipped":plan.skipped,
                }),
                undo: None,
            };
        }
        let plan = match plan_close(&response, &self.params, &self.policy) {
            Ok(plan) => plan,
            Err(message) => {
                return OrchStep::Error {
                    message,
                    hint: None,
                }
            }
        };
        if self.preview() || plan.tabs.is_empty() {
            return OrchStep::Complete {
                response: json!({
                    "dryRun": self.preview(), "txid":null,
                    "summary": {"closedTabs":0,"plannedTabs":plan.tabs.len(),"skippedTabs":plan.skipped.len()},
                    "skipped":plan.skipped,
                    "tabs":plan.tabs.iter().map(|tab| json!({
                        "tabId":tab.tab_id,"windowId":tab.window_id,"url":tab.url,"title":tab.title,
                    })).collect::<Vec<_>>(),
                }),
                undo: None,
            };
        }
        let ids: Vec<_> = plan.tabs.iter().map(|tab| tab.tab_id).collect();
        self.plan = Some(plan);
        OrchStep::SendPrimitive {
            action: "p:tab-remove".into(),
            params: json!({"tabIds":ids}),
        }
    }
}

fn plan_close(snapshot: &Value, params: &Value, policy: &Policy) -> Result<Plan, String> {
    let scope = select_tabs_by_scope(snapshot, params);
    if let Some(error) = scope.error {
        return Err(error);
    }
    let mut skipped = Vec::new();
    for id in params["tabIds"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_i64)
    {
        if !scope.tabs.iter().any(|tab| tab.tab_id == id) {
            skipped.push(json!({"tabId":id,"reason":"not_found"}));
        }
    }
    let tabs = scope
        .tabs
        .into_iter()
        .filter(|tab| {
            let expected_url = params["expectedUrls"]
                .get(tab.tab_id.to_string())
                .and_then(Value::as_str);
            let reason = policy.reason(tab).or_else(|| {
                expected_url
                    .filter(|url| tab.url.as_deref() != Some(*url))
                    .map(|_| "url_mismatch")
            });
            if let Some(reason) = reason {
                skipped.push(json!({"tabId":tab.tab_id,"reason":reason}));
                false
            } else {
                true
            }
        })
        .collect();
    Ok(Plan { tabs, skipped })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> Value {
        json!({"windows":[{"windowId":7,"focused":true,"tabs":[
            {"tabId":1,"url":"https://a.example","pinned":false},
            {"tabId":2,"url":"https://b.example","pinned":true}
        ]}]})
    }

    #[test]
    fn preview_and_explicit_empty_selection_never_issue_removal() {
        for params in [
            json!({"tabIds":[1]}),
            json!({"tabIds":[1],"confirmed":true,"dryRun":true}),
            json!({"tabIds":[],"confirmed":true}),
        ] {
            let mut orch = CloseOrchestration::new(&params);
            let _ = orch.start();
            let OrchStep::Complete { response, .. } = orch.step(snapshot()) else {
                panic!("must not remove")
            };
            assert_eq!(response["summary"]["closedTabs"], 0);
        }
    }

    #[test]
    fn close_plan_reports_missing_changed_and_protected_tabs() {
        let policy = serde_json::from_value(json!({"protect":{"pinned":true}})).unwrap();
        let plan = plan_close(
            &snapshot(),
            &json!({
                "tabIds":[1,2,3],"expectedUrls":{"1":"https://changed.example"}
            }),
            &policy,
        )
        .unwrap();
        assert!(plan.tabs.is_empty());
        assert_eq!(
            plan.skipped,
            vec![
                json!({"tabId":3,"reason":"not_found"}),
                json!({"tabId":1,"reason":"url_mismatch"}),
                json!({"tabId":2,"reason":"protected_pinned"}),
            ]
        );
    }
}
