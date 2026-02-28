use serde_json::{Map, Value};

use super::scope::select_tabs_by_scope;
use super::OrchStep;

/// Returns true for URLs that cannot be scripted (browser-internal pages).
pub(super) fn is_non_scriptable(url: &str) -> bool {
    url.starts_with("chrome://")
        || url.starts_with("edge://")
        || url.starts_with("about:")
        || url.starts_with("chrome-extension://")
        || url.starts_with("devtools://")
}

/// Orchestration for the `report` command.
///
/// p:snapshot → select tabs → for each scriptable tab: p:execute-script →
/// progress after each → aggregate → respond.
#[derive(Debug)]
pub(crate) struct ReportOrchestration {
    params: Value,
    state: Option<ReportState>,
}

#[derive(Debug)]
struct ReportState {
    tabs: Vec<ReportTab>,
    index: usize,
    results: Vec<Value>,
}

#[derive(Debug, Clone)]
struct ReportTab {
    tab_id: i64,
    window_id: i64,
    url: String,
    title: Option<String>,
    group_id: i64,
    group_title: Option<String>,
    active: Option<bool>,
    pinned: Option<bool>,
}

impl ReportOrchestration {
    pub(crate) fn new(params: &Value) -> Self {
        Self {
            params: params.clone(),
            state: None,
        }
    }
}

impl super::Orchestration for ReportOrchestration {
    fn start(&mut self) -> OrchStep {
        OrchStep::SendPrimitive {
            action: "p:snapshot".to_string(),
            params: Value::Object(Map::new()),
        }
    }

    fn step(&mut self, response: Value) -> OrchStep {
        if self.state.is_none() {
            return self.handle_snapshot(response);
        }
        self.handle_extract(response)
    }
}

impl ReportOrchestration {
    fn handle_snapshot(&mut self, snapshot: Value) -> OrchStep {
        let scope_result = select_tabs_by_scope(&snapshot, &self.params);
        if let Some(err) = scope_result.error {
            return OrchStep::Error {
                message: err,
                hint: None,
            };
        }

        let tabs: Vec<ReportTab> = scope_result
            .tabs
            .iter()
            .filter(|t| !is_non_scriptable(t.url.as_deref().unwrap_or("")))
            .map(|t| ReportTab {
                tab_id: t.tab_id,
                window_id: t.window_id,
                url: t.url.clone().unwrap_or_default(),
                title: t.title.clone(),
                group_id: t.group_id,
                group_title: t.group_title.clone(),
                active: t.active,
                pinned: t.pinned,
            })
            .collect();

        if tabs.is_empty() {
            return OrchStep::Complete {
                response: serde_json::json!({ "tabs": [] }),
                undo: None,
            };
        }

        let first_tab_id = tabs[0].tab_id;
        self.state = Some(ReportState {
            tabs,
            index: 0,
            results: Vec::new(),
        });

        OrchStep::SendPrimitive {
            action: "p:execute-script".to_string(),
            params: serde_json::json!({
                "tabId": first_tab_id,
                "signal": "description",
                "func": "extractDescription",
            }),
        }
    }

    fn handle_extract(&mut self, response: Value) -> OrchStep {
        let state = self.state.as_mut().unwrap();

        // Post-progress continuation: send next execute-script
        if response.is_null() {
            let tab = &state.tabs[state.index];
            return OrchStep::SendPrimitive {
                action: "p:execute-script".to_string(),
                params: serde_json::json!({
                    "tabId": tab.tab_id,
                    "signal": "description",
                    "func": "extractDescription",
                }),
            };
        }

        // Process execute-script response
        let description = response
            .get("result")
            .and_then(Value::as_array)
            .and_then(|arr| arr.first())
            .and_then(|r| r.get("result"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();

        let tab = &state.tabs[state.index];
        state.results.push(serde_json::json!({
            "tabId": tab.tab_id,
            "windowId": tab.window_id,
            "url": tab.url,
            "title": tab.title,
            "groupId": tab.group_id,
            "groupTitle": tab.group_title,
            "active": tab.active,
            "pinned": tab.pinned,
            "description": description,
        }));

        state.index += 1;

        if state.index >= state.tabs.len() {
            let final_results = std::mem::take(&mut state.results);
            return OrchStep::Complete {
                response: serde_json::json!({ "tabs": final_results }),
                undo: None,
            };
        }

        OrchStep::Progress {
            data: serde_json::json!({
                "done": state.index,
                "total": state.tabs.len(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host_impl::orchestrate::Orchestration;

    fn snapshot_with(tabs: Vec<Value>) -> Value {
        serde_json::json!({
            "windows": [{
                "windowId": 100,
                "focused": true,
                "tabs": tabs,
                "groups": []
            }]
        })
    }

    #[test]
    fn report_empty_scope() {
        let params = serde_json::json!({"windowId": 999});
        let mut orch = ReportOrchestration::new(&params);
        let _ = orch.start();
        let step = orch.step(snapshot_with(vec![]));
        let OrchStep::Complete { response, .. } = step else {
            panic!("expected Complete");
        };
        assert_eq!(response["tabs"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn report_skips_non_scriptable() {
        let params = serde_json::json!({});
        let mut orch = ReportOrchestration::new(&params);
        let _ = orch.start();
        let snap = snapshot_with(vec![
            serde_json::json!({"tabId": 1, "windowId": 100, "url": "chrome://settings", "title": "Settings", "groupId": -1}),
            serde_json::json!({"tabId": 2, "windowId": 100, "url": "https://example.com", "title": "Example", "groupId": -1}),
        ]);
        let step = orch.step(snap);
        let OrchStep::SendPrimitive { params, .. } = &step else {
            panic!("expected SendPrimitive, got {step:?}");
        };
        assert_eq!(params["tabId"], 2);
    }

    #[test]
    fn report_extracts_descriptions() {
        let params = serde_json::json!({});
        let mut orch = ReportOrchestration::new(&params);
        let _ = orch.start();
        let snap = snapshot_with(vec![
            serde_json::json!({"tabId": 1, "windowId": 100, "url": "https://a.com", "title": "A", "groupId": -1}),
            serde_json::json!({"tabId": 2, "windowId": 100, "url": "https://b.com", "title": "B", "groupId": -1}),
        ]);

        // Snapshot → execute-script for tab 1
        let step = orch.step(snap);
        assert!(matches!(&step, OrchStep::SendPrimitive { action, params }
            if action == "p:execute-script" && params["tabId"] == 1));

        // Tab 1 result → Progress
        let step = orch.step(serde_json::json!({"result": [{"result": "Description A"}]}));
        let OrchStep::Progress { data } = &step else {
            panic!("expected Progress, got {step:?}");
        };
        assert_eq!(data["done"], 1);
        assert_eq!(data["total"], 2);

        // Null (post-progress) → execute-script for tab 2
        let step = orch.step(Value::Null);
        assert!(matches!(&step, OrchStep::SendPrimitive { action, params }
            if action == "p:execute-script" && params["tabId"] == 2));

        // Tab 2 result → Complete
        let step = orch.step(serde_json::json!({"result": [{"result": "Description B"}]}));
        let OrchStep::Complete { response, .. } = step else {
            panic!("expected Complete");
        };
        let tabs = response["tabs"].as_array().unwrap();
        assert_eq!(tabs.len(), 2);
        assert_eq!(tabs[0]["description"], "Description A");
        assert_eq!(tabs[1]["description"], "Description B");
    }
}
