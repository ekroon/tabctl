use serde_json::{Map, Value};

use super::scope::select_tabs_by_scope;
use super::OrchStep;
use crate::host_impl::protocol::now_ms;

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
    emit_progress: bool,
}

#[derive(Debug, Clone)]
struct ReportTab {
    tab_id: i64,
    window_id: i64,
    url: String,
    title: Option<String>,
    group_id: i64,
    group_title: Option<String>,
    group_color: Option<String>,
    last_focused_at: Option<i64>,
    scriptable: bool,
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

fn build_report_entry(tab: &ReportTab, description: &str) -> Value {
    let mut entry = serde_json::json!({
        "tabId": tab.tab_id,
        "windowId": tab.window_id,
        "url": tab.url,
        "title": tab.title,
        "groupId": tab.group_id,
        "groupTitle": tab.group_title,
        "description": description,
    });
    if let Some(ref color) = tab.group_color {
        entry["groupColor"] = Value::String(color.clone());
    }
    if let Some(ts) = tab.last_focused_at {
        entry["lastFocusedAt"] = Value::Number(ts.into());
    }
    entry
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
            .map(|t| {
                let url = t.url.clone().unwrap_or_default();
                let scriptable = !is_non_scriptable(&url);
                ReportTab {
                    tab_id: t.tab_id,
                    window_id: t.window_id,
                    url,
                    title: t.title.clone(),
                    group_id: t.group_id,
                    group_title: t.group_title.clone(),
                    group_color: t.group_color.clone(),
                    last_focused_at: t.last_focused_at,
                    scriptable,
                }
            })
            .collect();

        if tabs.is_empty() {
            return OrchStep::Complete {
                response: serde_json::json!({
                    "generatedAt": now_ms(),
                    "entries": [],
                    "totals": { "tabs": 0 },
                }),
                undo: None,
            };
        }

        self.state = Some(ReportState {
            tabs: tabs.clone(),
            index: 0,
            results: Vec::new(),
            emit_progress: self
                .params
                .get("progress")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        });

        self.advance_to_next_scriptable()
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
                    "func": "extractPageMeta",
                }),
            };
        }

        // Process execute-script response — extractPageMeta returns {description, ...}
        let description = response
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();

        let tab = &state.tabs[state.index];
        state.results.push(build_report_entry(tab, &description));
        state.index += 1;

        self.advance_to_next_scriptable()
    }

    /// Advance through tabs, adding non-scriptable entries immediately,
    /// and sending p:execute-script for the next scriptable tab.
    fn advance_to_next_scriptable(&mut self) -> OrchStep {
        let state = self.state.as_mut().unwrap();

        // Add entries for any non-scriptable tabs at current position
        while state.index < state.tabs.len() && !state.tabs[state.index].scriptable {
            let tab = state.tabs[state.index].clone();
            state.results.push(build_report_entry(&tab, ""));
            state.index += 1;
        }

        if state.index >= state.tabs.len() {
            let entries = std::mem::take(&mut state.results);
            let total = entries.len();
            return OrchStep::Complete {
                response: serde_json::json!({
                    "generatedAt": now_ms(),
                    "entries": entries,
                    "totals": { "tabs": total },
                }),
                undo: None,
            };
        }

        let tab = &state.tabs[state.index];
        if state.index > 0 && state.emit_progress {
            OrchStep::Progress {
                data: serde_json::json!({
                    "done": state.index,
                    "total": state.tabs.len(),
                }),
            }
        } else {
            OrchStep::SendPrimitive {
                action: "p:execute-script".to_string(),
                params: serde_json::json!({
                    "tabId": tab.tab_id,
                    "func": "extractPageMeta",
                }),
            }
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
        assert_eq!(response["entries"].as_array().unwrap().len(), 0);
        assert_eq!(response["totals"]["tabs"], 0);
    }

    #[test]
    fn report_includes_non_scriptable_with_empty_desc() {
        let params = serde_json::json!({"progress": true});
        let mut orch = ReportOrchestration::new(&params);
        let _ = orch.start();
        let snap = snapshot_with(vec![
            serde_json::json!({"tabId": 1, "windowId": 100, "url": "chrome://settings", "title": "Settings", "groupId": -1}),
            serde_json::json!({"tabId": 2, "windowId": 100, "url": "https://example.com", "title": "Example", "groupId": -1}),
        ]);
        // Non-scriptable tab 1 auto-added → index advances to 1 → Progress
        let step = orch.step(snap);
        let OrchStep::Progress { .. } = &step else {
            panic!("expected Progress, got {step:?}");
        };
        // After progress → execute-script for scriptable tab 2
        let step = orch.step(Value::Null);
        let OrchStep::SendPrimitive { params, .. } = &step else {
            panic!("expected SendPrimitive, got {step:?}");
        };
        assert_eq!(params["tabId"], 2);
    }

    #[test]
    fn report_extracts_descriptions() {
        let params = serde_json::json!({"progress": true});
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
        let step = orch.step(serde_json::json!({"description": "Description A", "h1": ""}));
        let OrchStep::Progress { data } = &step else {
            panic!("expected Progress, got {step:?}");
        };
        assert_eq!(data["done"], 1);
        assert_eq!(data["total"], 2);

        // Null (post-progress) → execute-script for tab 2
        let step = orch.step(Value::Null);
        assert!(matches!(&step, OrchStep::SendPrimitive { action, params }
            if action == "p:execute-script" && params["tabId"] == 2));

        // Tab 2 result → Complete with entries + totals
        let step = orch.step(serde_json::json!({"description": "Description B", "h1": ""}));
        let OrchStep::Complete { response, .. } = step else {
            panic!("expected Complete");
        };
        let entries = response["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0]["description"], "Description A");
        assert_eq!(entries[1]["description"], "Description B");
        assert_eq!(response["totals"]["tabs"], 2);
        assert!(response["generatedAt"].as_u64().is_some());
    }
}
