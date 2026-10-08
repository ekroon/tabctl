use serde_json::{json, Value};
use std::sync::Arc;

use super::analyze::scoped_tab_values;
use super::scope::select_tabs_by_scope;
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
    tabs: Vec<Value>,
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
                    "summary": {"closedTabs":plan.tabs.len(),"plannedTabs":plan.tabs.len(),"skippedTabs":plan.skipped.len()},
                    "skipped":plan.skipped,
                    "tabs":plan.tabs,
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
                    "tabs":plan.tabs,
                }),
                undo: None,
            };
        }
        let ids: Vec<_> = plan.tabs.iter().map(|tab| tab["tabId"].clone()).collect();
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
    let tabs: Vec<_> = scope
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
    Ok(Plan {
        tabs: scoped_tab_values(snapshot, &tabs),
        skipped,
    })
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

    #[test]
    fn close_returns_full_pre_mutation_metadata_in_preview_and_confirmed_results() {
        let snapshot = json!({"windows":[{
            "windowId":7,"incognito":false,"focused":true,
            "groups":[{"groupId":42,"title":"Work","color":"green","collapsed":true}],
            "tabs":[{
                "tabId":1,"index":5,"groupId":42,"url":"https://a.example","title":"Original",
                "active":true,"pinned":true,"lastAccessedAt":1700000000000.5,
                "favIconUrl":"https://a.example/icon.png","status":"loading","audible":true,"discarded":true
            }]
        }]});
        let expected = json!([{
            "tabId":1,"windowId":7,"index":5,"groupId":42,"groupTitle":"Work",
            "groupColor":"green","groupCollapsed":true,"url":"https://a.example","title":"Original",
            "active":true,"pinned":true,"incognito":false,"lastAccessedAt":1700000000000.5,
            "favIconUrl":"https://a.example/icon.png","status":"loading","audible":true,"discarded":true
        }]);
        for params in [
            json!({"tabIds":[1]}),
            json!({"tabIds":[1],"confirmed":true,"dryRun":true}),
            json!({"tabIds":[1],"confirmed":true}),
        ] {
            let mut orch = CloseOrchestration::new(&params);
            let _ = orch.start();
            let step = orch.step(snapshot.clone());
            let confirmed = params["confirmed"] == true && params["dryRun"] != true;
            let result = if confirmed {
                let OrchStep::SendPrimitive { action, params } = step else {
                    panic!("expected close")
                };
                assert_eq!(action, "p:tab-remove");
                assert_eq!(params["tabIds"], json!([1]));
                orch.step(json!({"pinned":false,"index":0,"groupId":-1}))
            } else {
                step
            };
            let OrchStep::Complete { response, .. } = result else {
                panic!("expected result")
            };
            assert_eq!(response["tabs"], expected);
            assert_eq!(response["summary"]["plannedTabs"], 1);
            assert_eq!(response["summary"]["closedTabs"], usize::from(confirmed));
            assert_eq!(response["summary"]["skippedTabs"], 0);
        }
    }

    #[test]
    fn close_projection_excludes_skipped_tabs_and_counts_all_skipped_without_mutation() {
        let policy: Arc<Policy> =
            Arc::new(serde_json::from_value(json!({"protect":{"pinned":true}})).unwrap());
        for all_skipped in [false, true] {
            for confirmed in [false, true] {
                let ids = if all_skipped {
                    json!([2, 3])
                } else {
                    json!([1, 2, 3])
                };
                let mut orch =
                    CloseOrchestration::new(&json!({"tabIds":ids,"confirmed":confirmed}));
                orch.set_policy(Arc::clone(&policy));
                let _ = orch.start();
                let step = orch.step(snapshot());
                let result = if confirmed && !all_skipped {
                    let OrchStep::SendPrimitive { params, .. } = step else {
                        panic!("expected eligible close")
                    };
                    assert_eq!(params["tabIds"], json!([1]));
                    orch.step(json!({}))
                } else {
                    step
                };
                let OrchStep::Complete { response, .. } = result else {
                    panic!("must not remove skipped tabs")
                };
                assert_eq!(
                    response["summary"]["plannedTabs"],
                    usize::from(!all_skipped)
                );
                assert_eq!(
                    response["summary"]["closedTabs"],
                    usize::from(confirmed && !all_skipped)
                );
                assert_eq!(response["summary"]["skippedTabs"], 2);
                assert_eq!(
                    response["tabs"].as_array().unwrap().len(),
                    usize::from(!all_skipped)
                );
                assert_eq!(
                    response["skipped"],
                    json!([
                        {"tabId":3,"reason":"not_found"},{"tabId":2,"reason":"protected_pinned"}
                    ])
                );
            }
        }
    }
}
