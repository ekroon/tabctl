use serde_json::{Map, Value};
use std::collections::HashMap;

use super::report::is_non_scriptable;
use super::scope::select_tabs_by_scope;
use super::OrchStep;

/// Orchestration for the `inspect` command.
///
/// p:snapshot → select tabs → for each tab × signal: p:execute-script →
/// progress → aggregate → respond.
#[derive(Debug)]
pub(crate) struct InspectOrchestration {
    params: Value,
    state: Option<InspectState>,
}

#[derive(Debug)]
struct InspectState {
    /// Linearized list of (tab_index, signal_index) pairs.
    tasks: Vec<(usize, usize)>,
    task_index: usize,
    tabs: Vec<InspectTab>,
    signals: Vec<Signal>,
    /// Collected results keyed by tab_id → signal_name → value.
    results: HashMap<i64, Vec<(String, Value)>>,
    emit_progress: bool,
}

#[derive(Debug, Clone)]
struct InspectTab {
    tab_id: i64,
    window_id: i64,
    url: String,
    title: Option<String>,
}

#[derive(Debug, Clone)]
struct Signal {
    signal_type: String,
    name: String,
    selector: Option<String>,
    attr: Option<String>,
    timeout_ms: Option<i64>,
}

impl Signal {
    fn from_value(v: &Value) -> Option<Self> {
        let sig_type = v
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or("page-meta")
            .to_string();
        let name = v
            .get("name")
            .and_then(Value::as_str)
            .map(String::from)
            .unwrap_or_else(|| sig_type.clone());
        Some(Self {
            signal_type: sig_type,
            name,
            selector: v.get("selector").and_then(Value::as_str).map(String::from),
            attr: v.get("attr").and_then(Value::as_str).map(String::from),
            timeout_ms: None,
        })
    }

    fn from_selector_spec(spec: &Value) -> Option<Self> {
        let name = spec
            .get("name")
            .and_then(Value::as_str)
            .or_else(|| spec.get("selector").and_then(Value::as_str))?;
        Some(Self {
            signal_type: "selector".to_string(),
            name: name.to_string(),
            selector: spec
                .get("selector")
                .and_then(Value::as_str)
                .map(String::from),
            attr: spec.get("attr").and_then(Value::as_str).map(String::from),
            timeout_ms: None,
        })
    }

    fn default_page_meta() -> Self {
        Self {
            signal_type: "page-meta".to_string(),
            name: "page-meta".to_string(),
            selector: None,
            attr: None,
            timeout_ms: None,
        }
    }

    fn to_execute_params(&self, tab_id: i64) -> Value {
        match self.signal_type.as_str() {
            "selector" => {
                let selectors = vec![serde_json::json!({
                    "name": self.name,
                    "selector": self.selector,
                    "attr": self.attr.as_deref().unwrap_or("text"),
                })];
                let mut params = serde_json::json!({
                    "tabId": tab_id,
                    "func": "extractSelectorSignal",
                    "args": [selectors],
                });
                if let Some(ms) = self.timeout_ms {
                    params["timeoutMs"] = serde_json::json!(ms);
                }
                params
            }
            _ => {
                // page-meta or other → extractPageMeta
                let mut params = serde_json::json!({
                    "tabId": tab_id,
                    "func": "extractPageMeta",
                });
                if let Some(ms) = self.timeout_ms {
                    params["timeoutMs"] = serde_json::json!(ms);
                }
                params
            }
        }
    }
}

impl InspectOrchestration {
    pub(crate) fn new(params: &Value) -> Self {
        Self {
            params: params.clone(),
            state: None,
        }
    }
}

impl super::Orchestration for InspectOrchestration {
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

impl InspectOrchestration {
    fn handle_snapshot(&mut self, snapshot: Value) -> OrchStep {
        let scope_result = select_tabs_by_scope(&snapshot, &self.params);
        if let Some(err) = scope_result.error {
            return OrchStep::Error {
                message: err,
                hint: None,
            };
        }

        let tabs: Vec<InspectTab> = scope_result
            .tabs
            .iter()
            .filter(|t| !is_non_scriptable(t.url.as_deref().unwrap_or("")))
            .map(|t| InspectTab {
                tab_id: t.tab_id,
                window_id: t.window_id,
                url: t.url.clone().unwrap_or_default(),
                title: t.title.clone(),
            })
            .collect();

        let signals: Vec<Signal> = {
            if let Some(arr) = self.params.get("signals").and_then(Value::as_array) {
                let mut out: Vec<Signal> = Vec::new();
                for v in arr {
                    match v {
                        Value::Object(_) => {
                            if let Some(sig) = Signal::from_value(v) {
                                out.push(sig);
                            }
                        }
                        Value::String(id) => {
                            if id == "selector" {
                                if let Some(specs) =
                                    self.params.get("selectorSpecs").and_then(Value::as_array)
                                {
                                    for spec in specs {
                                        if let Some(sig) = Signal::from_selector_spec(spec) {
                                            out.push(sig);
                                        }
                                    }
                                }
                            } else {
                                out.push(Signal {
                                    signal_type: id.clone(),
                                    name: id.clone(),
                                    selector: None,
                                    attr: None,
                                    timeout_ms: None,
                                });
                            }
                        }
                        _ => {}
                    }
                }
                if out.is_empty() {
                    vec![Signal::default_page_meta()]
                } else {
                    out
                }
            } else {
                vec![Signal::default_page_meta()]
            }
        };

        if tabs.is_empty() {
            return OrchStep::Complete {
                response: serde_json::json!({
                    "totals": { "tabs": 0, "signals": signals.len(), "tasks": 0 },
                    "entries": [],
                }),
                undo: None,
            };
        }

        // Apply signal timeout to all signals
        let timeout_ms = self.params.get("signalTimeoutMs").and_then(Value::as_i64);
        let signals: Vec<Signal> = signals
            .into_iter()
            .map(|mut s| {
                s.timeout_ms = timeout_ms;
                s
            })
            .collect();

        // Build linearized task list: (tab_index, signal_index)
        let mut tasks = Vec::new();
        for ti in 0..tabs.len() {
            for si in 0..signals.len() {
                tasks.push((ti, si));
            }
        }

        let first_task = tasks[0];
        let first_tab_id = tabs[first_task.0].tab_id;
        let first_signal = &signals[first_task.1];
        let exec_params = first_signal.to_execute_params(first_tab_id);

        let emit_progress = self
            .params
            .get("progress")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        self.state = Some(InspectState {
            tasks,
            task_index: 0,
            tabs,
            signals,
            results: HashMap::new(),
            emit_progress,
        });

        OrchStep::SendPrimitive {
            action: "p:execute-script".to_string(),
            params: exec_params,
        }
    }

    fn handle_extract(&mut self, response: Value) -> OrchStep {
        let state = self.state.as_mut().unwrap();

        // Post-progress continuation
        if response.is_null() {
            let (ti, si) = state.tasks[state.task_index];
            let tab = &state.tabs[ti];
            let signal = &state.signals[si];
            return OrchStep::SendPrimitive {
                action: "p:execute-script".to_string(),
                params: signal.to_execute_params(tab.tab_id),
            };
        }

        // Process execute-script response
        let (ti, si) = state.tasks[state.task_index];
        let tab = &state.tabs[ti];
        let signal = &state.signals[si];

        state
            .results
            .entry(tab.tab_id)
            .or_default()
            .push((signal.name.clone(), response));

        state.task_index += 1;

        if state.task_index >= state.tasks.len() {
            return self.complete();
        }

        if state.emit_progress {
            OrchStep::Progress {
                data: serde_json::json!({
                    "done": state.task_index,
                    "total": state.tasks.len(),
                }),
            }
        } else {
            let (ti, si) = state.tasks[state.task_index];
            let tab = &state.tabs[ti];
            let signal = &state.signals[si];
            OrchStep::SendPrimitive {
                action: "p:execute-script".to_string(),
                params: signal.to_execute_params(tab.tab_id),
            }
        }
    }

    fn complete(&self) -> OrchStep {
        let state = self.state.as_ref().unwrap();

        let tab_results: Vec<Value> = state
            .tabs
            .iter()
            .map(|tab| {
                let signals_map: Map<String, Value> = state
                    .results
                    .get(&tab.tab_id)
                    .map(|entries| {
                        entries
                            .iter()
                            .map(|(name, val)| (name.clone(), val.clone()))
                            .collect()
                    })
                    .unwrap_or_default();

                serde_json::json!({
                    "tabId": tab.tab_id,
                    "windowId": tab.window_id,
                    "url": tab.url,
                    "title": tab.title,
                    "signals": Value::Object(signals_map),
                })
            })
            .collect();

        OrchStep::Complete {
            response: serde_json::json!({
                "totals": {
                    "tabs": state.tabs.len(),
                    "signals": state.signals.len(),
                    "tasks": state.tasks.len(),
                },
                "entries": tab_results,
            }),
            undo: None,
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
    fn inspect_empty_scope() {
        let params = serde_json::json!({"windowId": 999});
        let mut orch = InspectOrchestration::new(&params);
        let _ = orch.start();
        let step = orch.step(snapshot_with(vec![]));
        let OrchStep::Complete { response, .. } = step else {
            panic!("expected Complete");
        };
        assert_eq!(response["entries"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn inspect_single_signal() {
        let params = serde_json::json!({
            "signals": [{"type": "page-meta"}]
        });
        let mut orch = InspectOrchestration::new(&params);
        let _ = orch.start();

        let snap = snapshot_with(vec![
            serde_json::json!({"tabId": 1, "windowId": 100, "url": "https://a.com", "title": "A", "groupId": -1}),
        ]);

        // Snapshot → execute-script
        let step = orch.step(snap);
        let OrchStep::SendPrimitive { action, params } = &step else {
            panic!("expected SendPrimitive, got {step:?}");
        };
        assert_eq!(action, "p:execute-script");
        assert_eq!(params["tabId"], 1);
        assert_eq!(params["func"], "extractPageMeta");

        // Result → Complete
        let step = orch.step(serde_json::json!({"description": "Page desc"}));
        let OrchStep::Complete { response, .. } = step else {
            panic!("expected Complete");
        };
        let tabs = response["entries"].as_array().unwrap();
        assert_eq!(tabs.len(), 1);
        assert!(tabs[0]["signals"]["page-meta"].is_object());
    }

    #[test]
    fn inspect_multiple_signals() {
        let params = serde_json::json!({
            "signals": [
                {"type": "page-meta"},
                {"type": "selector", "name": "price", "selector": ".price", "attr": "textContent"}
            ],
            "progress": true
        });
        let mut orch = InspectOrchestration::new(&params);
        let _ = orch.start();

        let snap = snapshot_with(vec![
            serde_json::json!({"tabId": 1, "windowId": 100, "url": "https://a.com", "title": "A", "groupId": -1}),
        ]);

        // Snapshot → first signal (page-meta)
        let step = orch.step(snap);
        assert!(matches!(&step, OrchStep::SendPrimitive { action, params }
            if action == "p:execute-script" && params["func"] == "extractPageMeta"));

        // First signal result → Progress
        let step = orch.step(serde_json::json!({"description": "desc"}));
        assert!(matches!(&step, OrchStep::Progress { .. }));

        // Null → second signal (selector)
        let step = orch.step(Value::Null);
        let OrchStep::SendPrimitive { action, params } = &step else {
            panic!("expected SendPrimitive");
        };
        assert_eq!(action, "p:execute-script");
        assert_eq!(params["func"], "extractSelectorSignal");

        // Second signal result → Complete
        let step = orch.step(serde_json::json!({"price": "$9.99"}));
        let OrchStep::Complete { response, .. } = step else {
            panic!("expected Complete");
        };
        let tabs = response["entries"].as_array().unwrap();
        assert_eq!(tabs.len(), 1);
        assert!(tabs[0]["signals"]["page-meta"].is_object());
        assert!(tabs[0]["signals"]["price"].is_object());
    }

    #[test]
    fn inspect_no_progress_skips_progress_step() {
        let params = serde_json::json!({
            "signals": [
                {"type": "page-meta"},
                {"type": "selector", "name": "price", "selector": ".price", "attr": "textContent"}
            ]
        });
        let mut orch = InspectOrchestration::new(&params);
        let _ = orch.start();

        let snap = snapshot_with(vec![
            serde_json::json!({"tabId": 1, "windowId": 100, "url": "https://a.com", "title": "A", "groupId": -1}),
        ]);

        // Snapshot → first signal (page-meta)
        let step = orch.step(snap);
        assert!(matches!(&step, OrchStep::SendPrimitive { action, params }
            if action == "p:execute-script" && params["func"] == "extractPageMeta"));

        // First signal result → directly to SendPrimitive (no Progress)
        let step = orch.step(serde_json::json!({"description": "desc"}));
        let OrchStep::SendPrimitive { action, params } = &step else {
            panic!("expected SendPrimitive (no Progress), got {step:?}");
        };
        assert_eq!(action, "p:execute-script");
        assert_eq!(params["func"], "extractSelectorSignal");

        // Second signal result → Complete
        let step = orch.step(serde_json::json!({"price": "$9.99"}));
        let OrchStep::Complete { response, .. } = step else {
            panic!("expected Complete");
        };
        let tabs = response["entries"].as_array().unwrap();
        assert_eq!(tabs.len(), 1);
        assert!(tabs[0]["signals"]["page-meta"].is_object());
        assert!(tabs[0]["signals"]["price"].is_object());
    }

    #[test]
    fn inspect_parses_string_signals_with_selector_specs() {
        let params = serde_json::json!({
            "signals": ["page-meta", "selector"],
            "selectorSpecs": [
                {"name": "price", "selector": ".price", "attr": "text"},
                {"name": "link", "selector": "a[href]", "attr": "href-url"},
            ],
        });
        let mut orch = InspectOrchestration::new(&params);
        let _ = orch.start();
        let snap = snapshot_with(vec![
            serde_json::json!({"tabId": 1, "windowId": 100, "url": "https://example.com", "title": "Ex", "groupId": -1}),
        ]);
        let step = orch.step(snap);
        // Should have 3 tasks: page-meta + 2 selectors for 1 tab
        let OrchStep::SendPrimitive { action, params } = &step else {
            panic!("expected SendPrimitive, got {step:?}");
        };
        assert_eq!(action, "p:execute-script");
        // First task should be page-meta
        assert_eq!(params["func"], "extractPageMeta");
    }
}
