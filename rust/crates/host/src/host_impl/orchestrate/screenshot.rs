use serde_json::{Map, Value};

use super::report::is_non_scriptable;
use super::scope::select_tabs_by_scope;
use super::OrchStep;

/// Orchestration for the `screenshot` command.
///
/// p:snapshot → select tabs → for each tab: p:tab-query (save active) →
/// p:tab-update (switch) → p:screenshot-tile → p:tab-update (restore) →
/// progress → aggregate → respond.
#[derive(Debug)]
pub(crate) struct ScreenshotOrchestration {
    params: Value,
    state: Option<ScreenshotState>,
    phase: Phase,
}

#[derive(Debug)]
struct ScreenshotState {
    tabs: Vec<ScreenshotTab>,
    tab_index: usize,
    restore_tab_id: Option<i64>,
    results: Vec<Value>,
    options: Value,
}

#[derive(Debug, Clone)]
struct ScreenshotTab {
    tab_id: i64,
    window_id: i64,
    url: String,
    title: Option<String>,
}

#[derive(Debug)]
enum Phase {
    GetSnapshot,
    QueryActiveTab,
    SwitchToTab,
    CaptureTab,
    RestoreTab,
    AfterProgress,
}

impl ScreenshotOrchestration {
    pub(crate) fn new(params: &Value) -> Self {
        Self {
            params: params.clone(),
            state: None,
            phase: Phase::GetSnapshot,
        }
    }

    fn screenshot_options(&self) -> Value {
        let mut opts = Map::new();
        if let Some(mode) = self.params.get("mode").and_then(Value::as_str) {
            opts.insert("mode".to_string(), Value::String(mode.to_string()));
        } else {
            opts.insert("mode".to_string(), Value::String("viewport".to_string()));
        }
        if let Some(format) = self.params.get("format").and_then(Value::as_str) {
            opts.insert("format".to_string(), Value::String(format.to_string()));
        }
        if let Some(quality) = self.params.get("quality") {
            opts.insert("quality".to_string(), quality.clone());
        }
        if let Some(tile_max) = self.params.get("tileMaxDim") {
            opts.insert("tileMaxDim".to_string(), tile_max.clone());
        }
        if let Some(max_bytes) = self.params.get("maxBytes") {
            opts.insert("maxBytes".to_string(), max_bytes.clone());
        }
        Value::Object(opts)
    }
}

impl super::Orchestration for ScreenshotOrchestration {
    fn start(&mut self) -> OrchStep {
        OrchStep::SendPrimitive {
            action: "p:snapshot".to_string(),
            params: Value::Object(Map::new()),
        }
    }

    fn step(&mut self, response: Value) -> OrchStep {
        match self.phase {
            Phase::GetSnapshot => self.handle_snapshot(response),
            Phase::QueryActiveTab => self.handle_query_active(response),
            Phase::SwitchToTab => self.handle_switched(),
            Phase::CaptureTab => self.handle_captured(response),
            Phase::RestoreTab => self.handle_restored(),
            Phase::AfterProgress => self.start_next_tab(),
        }
    }
}

impl ScreenshotOrchestration {
    fn handle_snapshot(&mut self, snapshot: Value) -> OrchStep {
        let scope_result = select_tabs_by_scope(&snapshot, &self.params);
        if let Some(err) = scope_result.error {
            return OrchStep::Error {
                message: err,
                hint: None,
            };
        }

        let tabs: Vec<ScreenshotTab> = scope_result
            .tabs
            .iter()
            .filter(|t| !is_non_scriptable(t.url.as_deref().unwrap_or("")))
            .map(|t| ScreenshotTab {
                tab_id: t.tab_id,
                window_id: t.window_id,
                url: t.url.clone().unwrap_or_default(),
                title: t.title.clone(),
            })
            .collect();

        if tabs.is_empty() {
            return OrchStep::Complete {
                response: serde_json::json!({ "tabs": [] }),
                undo: None,
            };
        }

        let options = self.screenshot_options();

        self.state = Some(ScreenshotState {
            tabs,
            tab_index: 0,
            restore_tab_id: None,
            results: Vec::new(),
            options,
        });

        self.query_active_tab()
    }

    fn query_active_tab(&mut self) -> OrchStep {
        let state = self.state.as_ref().unwrap();
        let tab = &state.tabs[state.tab_index];
        self.phase = Phase::QueryActiveTab;
        OrchStep::SendPrimitive {
            action: "p:tab-query".to_string(),
            params: serde_json::json!({"query": {"windowId": tab.window_id, "active": true}}),
        }
    }

    fn handle_query_active(&mut self, response: Value) -> OrchStep {
        let state = self.state.as_mut().unwrap();
        let tab = &state.tabs[state.tab_index];

        // Find the currently active tab in the window
        let active_tab_id = response
            .as_array()
            .and_then(|tabs| tabs.first())
            .and_then(|t| t.get("id").and_then(Value::as_i64));

        state.restore_tab_id = active_tab_id;

        // If target tab is already active, skip the switch
        if active_tab_id == Some(tab.tab_id) {
            self.phase = Phase::CaptureTab;
            return OrchStep::SendPrimitive {
                action: "p:screenshot-tile".to_string(),
                params: serde_json::json!({
                    "tab": {"tabId": tab.tab_id, "windowId": tab.window_id},
                    "options": state.options,
                }),
            };
        }

        // Switch to target tab
        self.phase = Phase::SwitchToTab;
        OrchStep::SendPrimitive {
            action: "p:tab-update".to_string(),
            params: serde_json::json!({
                "tabId": tab.tab_id,
                "active": true,
            }),
        }
    }

    fn handle_switched(&mut self) -> OrchStep {
        let state = self.state.as_ref().unwrap();
        let tab = &state.tabs[state.tab_index];

        self.phase = Phase::CaptureTab;
        OrchStep::SendPrimitive {
            action: "p:screenshot-tile".to_string(),
            params: serde_json::json!({
                "tab": {"tabId": tab.tab_id, "windowId": tab.window_id},
                "options": state.options,
            }),
        }
    }

    fn handle_captured(&mut self, response: Value) -> OrchStep {
        let state = self.state.as_mut().unwrap();
        let tab = &state.tabs[state.tab_index];

        state.results.push(serde_json::json!({
            "tabId": tab.tab_id,
            "windowId": tab.window_id,
            "url": tab.url,
            "title": tab.title,
            "capture": response,
        }));

        // Restore previously active tab if we switched
        let restore_id = state.restore_tab_id;
        if let Some(restore_id) = restore_id {
            if restore_id != tab.tab_id {
                self.phase = Phase::RestoreTab;
                return OrchStep::SendPrimitive {
                    action: "p:tab-update".to_string(),
                    params: serde_json::json!({
                        "tabId": restore_id,
                        "active": true,
                    }),
                };
            }
        }

        self.advance_tab()
    }

    fn handle_restored(&mut self) -> OrchStep {
        self.advance_tab()
    }

    fn advance_tab(&mut self) -> OrchStep {
        let state = self.state.as_mut().unwrap();
        state.tab_index += 1;

        if state.tab_index >= state.tabs.len() {
            let final_results = std::mem::take(&mut state.results);
            return OrchStep::Complete {
                response: serde_json::json!({ "tabs": final_results }),
                undo: None,
            };
        }

        self.phase = Phase::AfterProgress;
        OrchStep::Progress {
            data: serde_json::json!({
                "done": state.tab_index,
                "total": state.tabs.len(),
            }),
        }
    }

    fn start_next_tab(&mut self) -> OrchStep {
        self.query_active_tab()
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
    fn screenshot_empty() {
        let params = serde_json::json!({"windowId": 999});
        let mut orch = ScreenshotOrchestration::new(&params);
        let _ = orch.start();
        let step = orch.step(snapshot_with(vec![]));
        let OrchStep::Complete { response, .. } = step else {
            panic!("expected Complete");
        };
        assert_eq!(response["tabs"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn screenshot_single_tab() {
        let params = serde_json::json!({"mode": "viewport"});
        let mut orch = ScreenshotOrchestration::new(&params);
        let _ = orch.start();

        let snap = snapshot_with(vec![serde_json::json!({
            "tabId": 1, "windowId": 100, "url": "https://a.com",
            "title": "A", "groupId": -1, "active": false
        })]);

        // Snapshot → tab-query to find active tab
        let step = orch.step(snap);
        let OrchStep::SendPrimitive { action, params } = &step else {
            panic!("expected SendPrimitive, got {step:?}");
        };
        assert_eq!(action, "p:tab-query");
        assert_eq!(params["query"]["windowId"], 100);

        // Query result: tab 5 is active → switch to tab 1
        let step = orch.step(serde_json::json!([{"id": 5, "windowId": 100, "active": true}]));
        let OrchStep::SendPrimitive { action, params } = &step else {
            panic!("expected tab-update, got {step:?}");
        };
        assert_eq!(action, "p:tab-update");
        assert_eq!(params["tabId"], 1);

        // Switched → capture
        let step = orch.step(serde_json::json!({"id": 1}));
        let OrchStep::SendPrimitive { action, params } = &step else {
            panic!("expected screenshot-tile, got {step:?}");
        };
        assert_eq!(action, "p:screenshot-tile");
        assert_eq!(params["tab"]["tabId"], 1);
        assert_eq!(params["options"]["mode"], "viewport");

        // Captured → restore tab 5
        let step = orch.step(serde_json::json!({"tiles": [{"data": "base64..."}], "tileCount": 1}));
        let OrchStep::SendPrimitive { action, params } = &step else {
            panic!("expected tab-update restore, got {step:?}");
        };
        assert_eq!(action, "p:tab-update");
        assert_eq!(params["tabId"], 5);

        // Restored → Complete
        let step = orch.step(serde_json::json!({"id": 5}));
        let OrchStep::Complete { response, .. } = step else {
            panic!("expected Complete");
        };
        let tabs = response["tabs"].as_array().unwrap();
        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs[0]["tabId"], 1);
        assert!(tabs[0]["capture"].is_object());
    }

    #[test]
    fn screenshot_skips_non_scriptable() {
        let params = serde_json::json!({});
        let mut orch = ScreenshotOrchestration::new(&params);
        let _ = orch.start();

        let snap = snapshot_with(vec![
            serde_json::json!({"tabId": 1, "windowId": 100, "url": "chrome://settings", "title": "Settings", "groupId": -1}),
            serde_json::json!({"tabId": 2, "windowId": 100, "url": "https://example.com", "title": "Example", "groupId": -1}),
        ]);

        // Snapshot → should skip chrome:// and start with tab 2
        let step = orch.step(snap);
        let OrchStep::SendPrimitive { action, params } = &step else {
            panic!("expected SendPrimitive, got {step:?}");
        };
        assert_eq!(action, "p:tab-query");
        // Only tab 2 should be in the list
        assert_eq!(params["query"]["windowId"], 100);
    }
}
