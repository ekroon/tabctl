use serde_json::{Map, Value};

use super::report::is_non_scriptable;
use super::scope::select_tabs_by_scope;
use super::OrchStep;

#[derive(Debug)]
pub(crate) struct ReadMarkdownOrchestration {
    params: Value,
    state: Option<ReadMarkdownState>,
}

#[derive(Debug)]
struct ReadMarkdownState {
    tabs: Vec<ReadMarkdownTab>,
    tab_index: usize,
    results: Vec<Value>,
    options: ReadMarkdownOptions,
}

#[derive(Debug, Clone)]
struct ReadMarkdownTab {
    tab_id: i64,
    window_id: i64,
    url: String,
    title: Option<String>,
}

#[derive(Debug, Clone)]
struct ReadMarkdownOptions {
    extract: bool,
    max_html_chars: Option<i64>,
    max_chars: Option<i64>,
    timeout_ms: Option<i64>,
}

impl ReadMarkdownOrchestration {
    pub(crate) fn new(params: &Value) -> Self {
        Self {
            params: params.clone(),
            state: None,
        }
    }
}

impl super::Orchestration for ReadMarkdownOrchestration {
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
        self.handle_markdown(response)
    }
}

impl ReadMarkdownOrchestration {
    fn handle_snapshot(&mut self, snapshot: Value) -> OrchStep {
        let scope_result = select_tabs_by_scope(&snapshot, &self.params);
        if let Some(err) = scope_result.error {
            return OrchStep::Error {
                message: err,
                hint: None,
            };
        }

        let tabs: Vec<ReadMarkdownTab> = scope_result
            .tabs
            .iter()
            .filter(|t| !is_non_scriptable(t.url.as_deref().unwrap_or("")))
            .map(|t| ReadMarkdownTab {
                tab_id: t.tab_id,
                window_id: t.window_id,
                url: t.url.clone().unwrap_or_default(),
                title: t.title.clone(),
            })
            .collect();

        if tabs.is_empty() {
            return OrchStep::Complete {
                response: serde_json::json!({
                    "totals": { "tabs": 0, "tasks": 0 },
                    "entries": [],
                }),
                undo: None,
            };
        }

        let options = ReadMarkdownOptions {
            extract: self
                .params
                .get("extract")
                .and_then(Value::as_bool)
                .unwrap_or(true),
            max_html_chars: self.params.get("maxHtmlChars").and_then(Value::as_i64),
            max_chars: self.params.get("maxChars").and_then(Value::as_i64),
            timeout_ms: self.params.get("timeoutMs").and_then(Value::as_i64),
        };

        let first_params = build_page_markdown_params(&tabs[0], &options);
        self.state = Some(ReadMarkdownState {
            tabs,
            tab_index: 0,
            results: Vec::new(),
            options,
        });

        OrchStep::SendPrimitive {
            action: "p:page-markdown".to_string(),
            params: first_params,
        }
    }

    fn handle_markdown(&mut self, response: Value) -> OrchStep {
        let state = self.state.as_mut().unwrap();
        let tab = state.tabs[state.tab_index].clone();
        state.results.push(build_result_entry(
            &tab,
            &state.options,
            if response.get("markdown").is_some() {
                Some(&response)
            } else {
                None
            },
        ));
        state.tab_index += 1;

        if state.tab_index >= state.tabs.len() {
            return self.complete();
        }

        let next_tab = &state.tabs[state.tab_index];
        OrchStep::SendPrimitive {
            action: "p:page-markdown".to_string(),
            params: build_page_markdown_params(next_tab, &state.options),
        }
    }

    fn complete(&self) -> OrchStep {
        let state = self.state.as_ref().unwrap();
        OrchStep::Complete {
            response: serde_json::json!({
                "totals": { "tabs": state.tabs.len(), "tasks": state.tabs.len() },
                "entries": state.results,
            }),
            undo: None,
        }
    }
}

fn build_page_markdown_params(tab: &ReadMarkdownTab, options: &ReadMarkdownOptions) -> Value {
    let mut p = serde_json::json!({
        "tabId": tab.tab_id,
        "extract": options.extract,
    });
    if let Some(v) = options.max_html_chars {
        p["maxHtmlChars"] = v.into();
    }
    if let Some(v) = options.max_chars {
        p["maxChars"] = v.into();
    }
    if let Some(v) = options.timeout_ms {
        p["timeoutMs"] = v.into();
    }
    p
}

fn build_result_entry(
    tab: &ReadMarkdownTab,
    options: &ReadMarkdownOptions,
    response: Option<&Value>,
) -> Value {
    if let Some(response) = response {
        serde_json::json!({
            "tabId": tab.tab_id,
            "windowId": tab.window_id,
            "url": tab.url,
            "title": tab.title,
            "markdown": response.get("markdown").and_then(|v| v.as_str()).unwrap_or(""),
            "chars": response.get("chars").and_then(|v| v.as_i64()).unwrap_or(0),
            "truncated": response.get("truncated").and_then(|v| v.as_bool()).unwrap_or(false),
            "extracted": response.get("extracted").and_then(|v| v.as_bool()).unwrap_or(true),
            "error": Value::Null,
        })
    } else {
        serde_json::json!({
            "tabId": tab.tab_id,
            "windowId": tab.window_id,
            "url": tab.url,
            "title": tab.title,
            "markdown": "",
            "chars": 0,
            "truncated": false,
            "extracted": options.extract,
            "error": "failed to read page",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host_impl::orchestrate::Orchestration;

    fn snapshot_with(tabs: Vec<Value>) -> Value {
        serde_json::json!({
            "windows": [{"windowId": 100, "focused": true, "tabs": tabs, "groups": []}]
        })
    }

    #[test]
    fn read_markdown_empty_scope() {
        let params = serde_json::json!({"windowId": 999});
        let mut orch = ReadMarkdownOrchestration::new(&params);
        let _ = orch.start();
        let step = orch.step(snapshot_with(vec![]));
        let OrchStep::Complete { response, .. } = step else {
            panic!("expected Complete")
        };
        assert_eq!(response["entries"].as_array().unwrap().len(), 0);
        assert_eq!(response["totals"]["tabs"], 0);
    }

    #[test]
    fn read_markdown_single_tab() {
        let params = serde_json::json!({"extract": true, "maxChars": 10000});
        let mut orch = ReadMarkdownOrchestration::new(&params);
        let _ = orch.start();
        let snap = snapshot_with(vec![
            serde_json::json!({"tabId": 1, "windowId": 100, "url": "https://a.com", "title": "A", "groupId": -1}),
        ]);
        let step = orch.step(snap);
        let OrchStep::SendPrimitive { action, params } = &step else {
            panic!("expected SendPrimitive, got {step:?}");
        };
        assert_eq!(action, "p:page-markdown");
        assert_eq!(params["tabId"], 1);
        assert_eq!(params["extract"], true);
        assert_eq!(params["maxChars"], 10000);

        let step = orch.step(
            serde_json::json!({"markdown": "# Hello", "chars": 7, "truncated": false, "extracted": true}),
        );
        let OrchStep::Complete { response, .. } = step else {
            panic!("expected Complete")
        };
        let entries = response["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["tabId"], 1);
        assert_eq!(entries[0]["markdown"], "# Hello");
        assert_eq!(entries[0]["chars"], 7);
        assert_eq!(entries[0]["truncated"], false);
    }

    #[test]
    fn read_markdown_multiple_tabs() {
        let params = serde_json::json!({});
        let mut orch = ReadMarkdownOrchestration::new(&params);
        let _ = orch.start();
        let snap = snapshot_with(vec![
            serde_json::json!({"tabId": 1, "windowId": 100, "url": "https://a.com", "title": "A", "groupId": -1}),
            serde_json::json!({"tabId": 2, "windowId": 100, "url": "https://b.com", "title": "B", "groupId": -1}),
        ]);
        let step = orch.step(snap);
        assert!(matches!(&step, OrchStep::SendPrimitive { action, params }
            if action == "p:page-markdown" && params["tabId"] == 1));

        let step = orch.step(
            serde_json::json!({"markdown": "Tab1", "chars": 4, "truncated": false, "extracted": true}),
        );
        assert!(matches!(&step, OrchStep::SendPrimitive { action, params }
            if action == "p:page-markdown" && params["tabId"] == 2));

        let step = orch.step(
            serde_json::json!({"markdown": "Tab2", "chars": 4, "truncated": false, "extracted": true}),
        );
        let OrchStep::Complete { response, undo } = step else {
            panic!("expected Complete")
        };
        assert!(undo.is_none());
        let entries = response["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(response["totals"]["tasks"], 2);
    }

    #[test]
    fn read_markdown_null_response_becomes_error_entry() {
        let params = serde_json::json!({});
        let mut orch = ReadMarkdownOrchestration::new(&params);
        let _ = orch.start();
        let snap = snapshot_with(vec![
            serde_json::json!({"tabId": 1, "windowId": 100, "url": "https://a.com", "title": "A", "groupId": -1}),
        ]);
        let _ = orch.step(snap);
        let step = orch.step(serde_json::Value::Null);
        let OrchStep::Complete { response, .. } = step else {
            panic!("expected Complete")
        };
        let entry = &response["entries"][0];
        assert_eq!(entry["markdown"], "");
        assert!(!entry["error"].is_null());
    }

    #[test]
    fn read_markdown_skips_non_scriptable_tabs() {
        let params = serde_json::json!({});
        let mut orch = ReadMarkdownOrchestration::new(&params);
        let _ = orch.start();
        let snap = snapshot_with(vec![
            serde_json::json!({"tabId": 1, "windowId": 100, "url": "chrome://settings", "title": "Settings", "groupId": -1}),
            serde_json::json!({"tabId": 2, "windowId": 100, "url": "https://a.com", "title": "A", "groupId": -1}),
        ]);
        let step = orch.step(snap);
        let OrchStep::SendPrimitive { params, .. } = &step else {
            panic!("expected SendPrimitive")
        };
        assert_eq!(params["tabId"], 2);
    }
}
