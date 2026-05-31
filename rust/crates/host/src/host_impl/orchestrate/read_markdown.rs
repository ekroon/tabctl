use html_to_markdown_rs::{convert, ConversionOptions, PreprocessingOptions};
use serde_json::{Map, Value};

use super::report::is_non_scriptable;
use super::scope::select_tabs_by_scope;
use super::OrchStep;

const STATUS_READ: &str = "READ";
const STATUS_EMPTY: &str = "EMPTY";
const STATUS_UNSUPPORTED_URL: &str = "UNSUPPORTED_URL";
const STATUS_NOT_LOADED: &str = "NOT_LOADED";
const STATUS_INJECTION_FAILED: &str = "INJECTION_FAILED";
const STATUS_EXTRACTION_FAILED: &str = "EXTRACTION_FAILED";
const STATUS_TIMED_OUT: &str = "TIMED_OUT";
const STATUS_PROTECTED: &str = "PROTECTED";

#[derive(Debug)]
pub(crate) struct ReadMarkdownOrchestration {
    params: Value,
    state: Option<ReadMarkdownState>,
}

#[derive(Debug)]
struct ReadMarkdownState {
    tabs: Vec<ReadMarkdownTab>,
    tab_index: usize,
    in_flight_index: Option<usize>,
    tasks: usize,
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
        self.handle_html(response)
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

        self.state = Some(ReadMarkdownState {
            tabs,
            tab_index: 0,
            in_flight_index: None,
            tasks: 0,
            results: Vec::new(),
            options,
        });

        self.next_step()
    }

    fn handle_html(&mut self, response: Value) -> OrchStep {
        let state = self.state.as_mut().unwrap();
        let Some(in_flight_index) = state.in_flight_index.take() else {
            return OrchStep::Error {
                message: "readTabs received page HTML without an in-flight tab".to_string(),
                hint: None,
            };
        };
        let tab = state.tabs[in_flight_index].clone();
        state
            .results
            .push(build_result_entry(&tab, &state.options, Some(&response)));
        self.next_step()
    }

    fn next_step(&mut self) -> OrchStep {
        loop {
            let state = self.state.as_mut().unwrap();
            if state.tab_index >= state.tabs.len() {
                return self.complete();
            }

            let tab = state.tabs[state.tab_index].clone();
            if is_non_scriptable(&tab.url) {
                state
                    .results
                    .push(build_unsupported_entry(&tab, &state.options));
                state.tab_index += 1;
                continue;
            }

            let params = build_page_html_params(&tab, &state.options);
            state.in_flight_index = Some(state.tab_index);
            state.tab_index += 1;
            state.tasks += 1;
            return OrchStep::SendPrimitive {
                action: "p:page-html".to_string(),
                params,
            };
        }
    }

    fn complete(&self) -> OrchStep {
        let state = self.state.as_ref().unwrap();
        OrchStep::Complete {
            response: serde_json::json!({
                "totals": { "tabs": state.tabs.len(), "tasks": state.tasks },
                "entries": state.results,
            }),
            undo: None,
        }
    }
}

fn build_page_html_params(tab: &ReadMarkdownTab, options: &ReadMarkdownOptions) -> Value {
    let mut p = serde_json::json!({
        "tabId": tab.tab_id,
    });
    if let Some(v) = options.max_html_chars {
        p["maxHtmlChars"] = v.into();
    }
    if let Some(v) = options.timeout_ms {
        p["timeoutMs"] = v.into();
    }
    p
}

fn build_unsupported_entry(tab: &ReadMarkdownTab, options: &ReadMarkdownOptions) -> Value {
    base_entry(
        tab,
        options,
        STATUS_UNSUPPORTED_URL,
        "",
        0,
        false,
        Some("unsupported URL for content-script extraction"),
        Some("unsupported_url"),
        diagnostics(0, 0, None, false),
    )
}

fn build_result_entry(
    tab: &ReadMarkdownTab,
    options: &ReadMarkdownOptions,
    response: Option<&Value>,
) -> Value {
    let Some(response) = response else {
        return base_entry(
            tab,
            options,
            STATUS_EXTRACTION_FAILED,
            "",
            0,
            false,
            Some("failed to read page"),
            Some("missing_response"),
            diagnostics(0, 0, None, false),
        );
    };

    let source_html_chars = response
        .get("sourceHtmlChars")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let source_text_chars = response
        .get("sourceTextChars")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let document_ready_state = response
        .get("documentReadyState")
        .and_then(Value::as_str)
        .map(str::to_string);
    let truncated_html = response
        .get("truncatedHtml")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let diag = diagnostics(
        source_html_chars,
        source_text_chars,
        document_ready_state.as_deref(),
        truncated_html,
    );

    let response_status = response
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or(STATUS_EXTRACTION_FAILED);
    if response_status != STATUS_READ {
        let status = normalize_status(response_status);
        let error = response
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or_else(|| default_error(status));
        return base_entry(
            tab,
            options,
            status,
            "",
            0,
            false,
            Some(error),
            Some(empty_reason_for_status(status)),
            diag,
        );
    }

    let html = response.get("html").and_then(Value::as_str).unwrap_or("");
    if html.is_empty() {
        let status = if document_ready_state.as_deref() == Some("loading") {
            STATUS_NOT_LOADED
        } else {
            STATUS_EMPTY
        };
        return base_entry(
            tab,
            options,
            status,
            "",
            0,
            false,
            Some(default_error(status)),
            Some(empty_reason_for_status(status)),
            diag,
        );
    }

    let conversion = convert(html, Some(conversion_options(options.extract)));
    let full_markdown = match conversion {
        Ok(result) => result.content.unwrap_or_default(),
        Err(err) => {
            let message = format!("Markdown conversion failed: {err}");
            return base_entry(
                tab,
                options,
                STATUS_EXTRACTION_FAILED,
                "",
                0,
                false,
                Some(&message),
                Some("conversion_failed"),
                diag,
            );
        }
    };
    let (markdown, chars, truncated) = truncate_markdown(&full_markdown, options.max_chars);

    if markdown.trim().is_empty() {
        let status = if source_text_chars > 0 {
            STATUS_EXTRACTION_FAILED
        } else {
            STATUS_EMPTY
        };
        return base_entry(
            tab,
            options,
            status,
            "",
            0,
            false,
            Some(default_error(status)),
            Some(if source_text_chars > 0 {
                "conversion_produced_empty_markdown"
            } else {
                "page_text_empty"
            }),
            diag,
        );
    }

    base_entry(
        tab,
        options,
        STATUS_READ,
        &markdown,
        chars,
        truncated,
        None,
        None,
        diag,
    )
}

fn conversion_options(extract: bool) -> ConversionOptions {
    ConversionOptions {
        preprocessing: PreprocessingOptions {
            enabled: extract,
            ..PreprocessingOptions::default()
        },
        ..ConversionOptions::default()
    }
}

fn truncate_markdown(markdown: &str, max_chars: Option<i64>) -> (String, i64, bool) {
    let max_chars = max_chars.unwrap_or(50_000).clamp(1, 200_000) as usize;
    let total_chars = markdown.chars().count();
    if total_chars > max_chars {
        (
            markdown.chars().take(max_chars).collect(),
            max_chars as i64,
            true,
        )
    } else {
        (markdown.to_string(), total_chars as i64, false)
    }
}

fn normalize_status(status: &str) -> &'static str {
    match status {
        STATUS_TIMED_OUT => STATUS_TIMED_OUT,
        STATUS_PROTECTED => STATUS_PROTECTED,
        STATUS_NOT_LOADED => STATUS_NOT_LOADED,
        STATUS_INJECTION_FAILED => STATUS_INJECTION_FAILED,
        STATUS_UNSUPPORTED_URL => STATUS_UNSUPPORTED_URL,
        STATUS_EMPTY => STATUS_EMPTY,
        _ => STATUS_EXTRACTION_FAILED,
    }
}

fn default_error(status: &str) -> &'static str {
    match status {
        STATUS_EMPTY => "page produced empty Markdown",
        STATUS_UNSUPPORTED_URL => "unsupported URL for content-script extraction",
        STATUS_PROTECTED => "browser blocked access to this tab",
        STATUS_NOT_LOADED => "page was still loading",
        STATUS_INJECTION_FAILED => "content-script injection failed",
        STATUS_TIMED_OUT => "page extraction timed out",
        _ => "page extraction failed",
    }
}

fn empty_reason_for_status(status: &str) -> &'static str {
    match status {
        STATUS_EMPTY => "page_text_empty",
        STATUS_UNSUPPORTED_URL => "unsupported_url",
        STATUS_PROTECTED => "protected_page",
        STATUS_NOT_LOADED => "document_not_loaded",
        STATUS_INJECTION_FAILED => "injection_failed",
        STATUS_TIMED_OUT => "timed_out",
        _ => "extraction_failed",
    }
}

fn diagnostics(
    source_html_chars: i64,
    source_text_chars: i64,
    document_ready_state: Option<&str>,
    truncated_html: bool,
) -> Value {
    serde_json::json!({
        "sourceHtmlChars": source_html_chars,
        "sourceTextChars": source_text_chars,
        "documentReadyState": document_ready_state,
        "truncatedHtml": truncated_html,
    })
}

#[allow(clippy::too_many_arguments)]
fn base_entry(
    tab: &ReadMarkdownTab,
    options: &ReadMarkdownOptions,
    status: &str,
    markdown: &str,
    chars: i64,
    truncated: bool,
    error: Option<&str>,
    empty_reason: Option<&str>,
    diagnostics: Value,
) -> Value {
    serde_json::json!({
        "tabId": tab.tab_id,
        "windowId": tab.window_id,
        "url": tab.url,
        "title": tab.title,
        "markdown": markdown,
        "chars": chars,
        "truncated": truncated,
        "extracted": options.extract,
        "status": status,
        "emptyReason": empty_reason,
        "diagnostics": diagnostics,
        "error": error,
    })
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

    fn html_response(html: &str, source_text_chars: i64) -> Value {
        serde_json::json!({
            "status": "READ",
            "html": html,
            "sourceHtmlChars": html.chars().count(),
            "sourceTextChars": source_text_chars,
            "documentReadyState": "complete",
            "truncatedHtml": false,
            "error": null
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
        assert_eq!(action, "p:page-html");
        assert_eq!(params["tabId"], 1);
        assert_eq!(params["maxChars"], Value::Null);

        let step = orch.step(html_response("<h1>Hello</h1><p>World.</p>", 12));
        let OrchStep::Complete { response, .. } = step else {
            panic!("expected Complete")
        };
        let entries = response["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["tabId"], 1);
        assert_eq!(entries[0]["status"], STATUS_READ);
        assert!(entries[0]["markdown"].as_str().unwrap().contains("Hello"));
        assert_eq!(entries[0]["truncated"], false);
    }

    #[test]
    fn read_markdown_multiple_tabs_preserves_order() {
        let params = serde_json::json!({});
        let mut orch = ReadMarkdownOrchestration::new(&params);
        let _ = orch.start();
        let snap = snapshot_with(vec![
            serde_json::json!({"tabId": 1, "windowId": 100, "url": "https://a.com", "title": "A", "groupId": -1}),
            serde_json::json!({"tabId": 2, "windowId": 100, "url": "https://b.com", "title": "B", "groupId": -1}),
        ]);
        let step = orch.step(snap);
        assert!(matches!(&step, OrchStep::SendPrimitive { action, params }
            if action == "p:page-html" && params["tabId"] == 1));

        let step = orch.step(html_response("<p>Tab1</p>", 4));
        assert!(matches!(&step, OrchStep::SendPrimitive { action, params }
            if action == "p:page-html" && params["tabId"] == 2));

        let step = orch.step(html_response("<p>Tab2</p>", 4));
        let OrchStep::Complete { response, undo } = step else {
            panic!("expected Complete")
        };
        assert!(undo.is_none());
        let entries = response["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0]["tabId"], 1);
        assert_eq!(entries[1]["tabId"], 2);
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
        assert_eq!(entry["status"], STATUS_EXTRACTION_FAILED);
        assert!(!entry["error"].is_null());
    }

    #[test]
    fn read_markdown_keeps_non_scriptable_tabs_as_unsupported_entries() {
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

        let step = orch.step(html_response("<p>A</p>", 1));
        let OrchStep::Complete { response, .. } = step else {
            panic!("expected Complete")
        };
        assert_eq!(response["totals"]["tabs"], 2);
        assert_eq!(response["totals"]["tasks"], 1);
        assert_eq!(response["entries"][0]["status"], STATUS_UNSUPPORTED_URL);
        assert_eq!(response["entries"][1]["status"], STATUS_READ);
    }

    #[test]
    fn read_markdown_empty_conversion_is_not_success() {
        let params = serde_json::json!({});
        let mut orch = ReadMarkdownOrchestration::new(&params);
        let _ = orch.start();
        let snap = snapshot_with(vec![
            serde_json::json!({"tabId": 1, "windowId": 100, "url": "https://a.com", "title": "A", "groupId": -1}),
        ]);
        let _ = orch.step(snap);
        let step = orch.step(html_response(
            "<html><body><script>1</script></body></html>",
            42,
        ));
        let OrchStep::Complete { response, .. } = step else {
            panic!("expected Complete")
        };
        let entry = &response["entries"][0];
        assert_eq!(entry["status"], STATUS_EXTRACTION_FAILED);
        assert!(!entry["error"].is_null());
        assert_eq!(entry["diagnostics"]["sourceTextChars"], 42);
    }

    #[test]
    fn read_markdown_timeout_status_is_preserved() {
        let params = serde_json::json!({});
        let mut orch = ReadMarkdownOrchestration::new(&params);
        let _ = orch.start();
        let snap = snapshot_with(vec![
            serde_json::json!({"tabId": 1, "windowId": 100, "url": "https://a.com", "title": "A", "groupId": -1}),
        ]);
        let _ = orch.step(snap);
        let step = orch.step(serde_json::json!({
            "status": "TIMED_OUT",
            "error": "Timed out after 10ms",
            "sourceHtmlChars": 0,
            "sourceTextChars": 0,
            "documentReadyState": null,
            "truncatedHtml": false
        }));
        let OrchStep::Complete { response, .. } = step else {
            panic!("expected Complete")
        };
        let entry = &response["entries"][0];
        assert_eq!(entry["status"], STATUS_TIMED_OUT);
        assert_eq!(entry["emptyReason"], "timed_out");
    }

    #[test]
    fn truncate_markdown_uses_character_boundaries() {
        let (markdown, chars, truncated) = truncate_markdown("aé🙂b", Some(3));
        assert_eq!(markdown, "aé🙂");
        assert_eq!(chars, 3);
        assert!(truncated);
    }
}
