use html_to_markdown_rs::{convert, ConversionOptions, PreprocessingOptions};
use serde_json::{Map, Value};

use crate::host_impl::page_cache::{OpenTabCacheKey, PageCache, PageCacheEntry, PageCacheLookup};
use crate::host_impl::protocol::{log_line, now_ms};

use super::report::is_non_scriptable;
use super::scope::select_tabs_by_scope;
use super::{OrchStep, OrchestrationContext};

const STATUS_READ: &str = "READ";
const STATUS_CACHED: &str = "CACHED";
const STATUS_EMPTY: &str = "EMPTY";
const STATUS_UNSUPPORTED_URL: &str = "UNSUPPORTED_URL";
const STATUS_NOT_LOADED: &str = "NOT_LOADED";
const STATUS_INJECTION_FAILED: &str = "INJECTION_FAILED";
const STATUS_EXTRACTION_FAILED: &str = "EXTRACTION_FAILED";
const STATUS_TIMED_OUT: &str = "TIMED_OUT";
const STATUS_PROTECTED: &str = "PROTECTED";
const MAX_HTML_CHARS_PER_READ: i64 = 1_500_000;

#[derive(Debug)]
pub(crate) struct ReadMarkdownOrchestration {
    params: Value,
    state: Option<ReadMarkdownState>,
    context: OrchestrationContext,
}

#[derive(Debug)]
struct ReadMarkdownState {
    tabs: Vec<ReadMarkdownTab>,
    tab_index: usize,
    in_flight_index: Option<usize>,
    tasks: usize,
    results: Vec<Value>,
    options: ReadMarkdownOptions,
    cache: Option<PageCache>,
}

#[derive(Debug, Clone)]
struct ReadMarkdownTab {
    tab_id: i64,
    window_id: i64,
    url: String,
    title: Option<String>,
    incognito: bool,
}

#[derive(Debug, Clone)]
struct ReadMarkdownOptions {
    extract: bool,
    max_html_chars: Option<i64>,
    max_chars: Option<i64>,
    timeout_ms: Option<i64>,
}

impl ReadMarkdownOrchestration {
    pub(crate) fn new(params: &Value, context: OrchestrationContext) -> Self {
        Self {
            params: params.clone(),
            state: None,
            context,
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
                incognito: t.incognito,
            })
            .collect();

        let mut cache = self.context.page_cache_path.as_deref().map(PageCache::load);
        if let Some(cache) = cache.as_mut() {
            let open_tabs = open_cache_keys_from_snapshot(&snapshot);
            cache.prune_to_open_tabs(self.context.profile_name.as_deref(), &open_tabs);
        }

        if tabs.is_empty() {
            self.save_cache_if_dirty(cache.as_mut());
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
            cache,
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
        let entry = build_result_entry(&tab, &state.options, Some(&response));
        if entry["status"] == STATUS_READ {
            maybe_store_success(
                state.cache.as_mut(),
                self.context.profile_name.as_deref(),
                &tab,
                &response,
            );
            state.results.push(entry);
        } else if let Some(cached) = eligible_cache_fallback(
            &tab,
            &response,
            state.cache.as_ref(),
            self.context.profile_name.as_deref(),
        ) {
            state.results.push(build_cached_entry(
                &tab,
                &state.options,
                &cached.entry,
                cached.match_mode.as_str(),
                now_ms() as i64,
            ));
        } else {
            state.results.push(entry);
        }
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

    fn complete(&mut self) -> OrchStep {
        let (tabs_len, tasks, results, mut cache) = {
            let state = self.state.as_mut().unwrap();
            (
                state.tabs.len(),
                state.tasks,
                state.results.clone(),
                state.cache.take(),
            )
        };
        self.save_cache_if_dirty(cache.as_mut());
        OrchStep::Complete {
            response: serde_json::json!({
                "totals": { "tabs": tabs_len, "tasks": tasks },
                "entries": results,
            }),
            undo: None,
        }
    }

    fn save_cache_if_dirty(&self, cache: Option<&mut PageCache>) {
        if let (Some(path), Some(cache)) = (&self.context.page_cache_path, cache) {
            if let Err(err) = cache.save_if_dirty(path) {
                log_line(&format!("readTabs page cache save failed: {err}"));
            }
        }
    }
}

fn build_page_html_params(tab: &ReadMarkdownTab, options: &ReadMarkdownOptions) -> Value {
    let mut p = serde_json::json!({
        "tabId": tab.tab_id,
        "expectedUrl": tab.url,
    });
    if let Some(v) = options.max_html_chars {
        p["maxHtmlChars"] = v.into();
    }
    if let Some(v) = options.timeout_ms {
        p["timeoutMs"] = v.into();
    }
    p
}

fn open_cache_keys_from_snapshot(snapshot: &Value) -> Vec<OpenTabCacheKey> {
    let mut keys = Vec::new();
    for window in snapshot
        .get("windows")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let window_incognito = window
            .get("incognito")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        for tab in window
            .get("tabs")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let tab_id = tab.get("tabId").and_then(Value::as_i64);
            let url = tab.get("url").and_then(Value::as_str);
            let incognito = tab
                .get("incognito")
                .and_then(Value::as_bool)
                .unwrap_or(window_incognito);
            if let (Some(tab_id), Some(url)) = (tab_id, url) {
                if !incognito && !url.is_empty() && !is_non_scriptable(url) {
                    keys.push(OpenTabCacheKey::new(tab_id, url));
                }
            }
        }
    }
    keys
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
        "source": "live",
        "cachedAt": null,
        "cacheAgeMs": null,
    })
}

fn cache_diagnostics_with_truncation(
    entry: &PageCacheEntry,
    cache_match: &str,
    now: i64,
    truncated_html: bool,
) -> Value {
    serde_json::json!({
        "sourceHtmlChars": entry.source_html_chars,
        "sourceTextChars": entry.source_text_chars,
        "documentReadyState": entry.document_ready_state,
        "truncatedHtml": truncated_html,
        "source": "cache",
        "cachedAt": entry.captured_at,
        "cacheAgeMs": now.saturating_sub(entry.captured_at),
        "cacheMatch": cache_match,
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
        "cached": status == STATUS_CACHED,
        "emptyReason": empty_reason,
        "diagnostics": diagnostics,
        "error": error,
    })
}

fn maybe_store_success(
    cache: Option<&mut PageCache>,
    profile: Option<&str>,
    tab: &ReadMarkdownTab,
    response: &Value,
) {
    let Some(cache) = cache else {
        return;
    };
    let html = response.get("html").and_then(Value::as_str).unwrap_or("");
    let source_html_chars = response
        .get("sourceHtmlChars")
        .and_then(Value::as_i64)
        .unwrap_or_else(|| html.chars().count() as i64);
    let source_text_chars = response
        .get("sourceTextChars")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let document_ready_state = response.get("documentReadyState").and_then(Value::as_str);
    let truncated_html = response
        .get("truncatedHtml")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    cache.store_success(
        profile,
        tab.tab_id,
        &tab.url,
        tab.title.as_deref(),
        html,
        source_html_chars,
        source_text_chars,
        document_ready_state,
        truncated_html,
        tab.incognito,
        now_ms() as i64,
    );
}

fn eligible_cache_fallback(
    tab: &ReadMarkdownTab,
    response: &Value,
    cache: Option<&PageCache>,
    profile: Option<&str>,
) -> Option<PageCacheLookup> {
    if tab.incognito || is_non_scriptable(&tab.url) {
        return None;
    }
    let status = response.get("status").and_then(Value::as_str)?;
    if !matches!(
        status,
        STATUS_PROTECTED
            | STATUS_TIMED_OUT
            | STATUS_NOT_LOADED
            | STATUS_INJECTION_FAILED
            | STATUS_EXTRACTION_FAILED
    ) {
        return None;
    }
    cache?.lookup_open_tab(profile, tab.tab_id, &tab.url)
}

fn build_cached_entry(
    tab: &ReadMarkdownTab,
    options: &ReadMarkdownOptions,
    cached: &PageCacheEntry,
    cache_match: &str,
    now: i64,
) -> Value {
    let (html, truncated_html_for_request) =
        truncate_cached_html_for_request(&cached.html, options.max_html_chars);
    let conversion = convert(&html, Some(conversion_options(options.extract)));
    let full_markdown = match conversion {
        Ok(result) => result.content.unwrap_or_default(),
        Err(_) => return build_result_entry(tab, options, None),
    };
    let (markdown, chars, truncated) = truncate_markdown(&full_markdown, options.max_chars);
    if markdown.trim().is_empty() {
        return build_result_entry(tab, options, None);
    }
    base_entry(
        tab,
        options,
        STATUS_CACHED,
        &markdown,
        chars,
        truncated,
        None,
        None,
        cache_diagnostics_with_truncation(cached, cache_match, now, truncated_html_for_request),
    )
}

fn truncate_cached_html_for_request(html: &str, max_html_chars: Option<i64>) -> (String, bool) {
    let Some(max_html_chars) = max_html_chars else {
        return (html.to_string(), false);
    };
    let max_html_chars = max_html_chars.clamp(1, MAX_HTML_CHARS_PER_READ) as usize;
    let total_chars = html.chars().count();
    if total_chars > max_html_chars {
        (html.chars().take(max_html_chars).collect(), true)
    } else {
        (html.to_string(), false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host_impl::orchestrate::Orchestration;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn cache_path(name: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("target")
            .join("read-markdown-cache-tests")
            .join(format!("{}-{}-{}", name, std::process::id(), id));
        let _ = std::fs::remove_dir_all(&path);
        path
    }

    fn context_with_cache(path: PathBuf) -> OrchestrationContext {
        OrchestrationContext {
            page_cache_path: Some(path),
            profile_name: Some("test-profile".to_string()),
        }
    }

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

    fn status_response(status: &str) -> Value {
        serde_json::json!({
            "status": status,
            "error": null,
            "sourceHtmlChars": 0,
            "sourceTextChars": 0,
            "documentReadyState": null,
            "truncatedHtml": false
        })
    }

    #[test]
    fn read_markdown_empty_scope() {
        let params = serde_json::json!({"windowId": 999});
        let mut orch = ReadMarkdownOrchestration::new(&params, OrchestrationContext::default());
        let _ = orch.start();
        let step = orch.step(snapshot_with(vec![]));
        let OrchStep::Complete { response, .. } = step else {
            panic!("expected Complete")
        };
        assert_eq!(response["entries"].as_array().unwrap().len(), 0);
        assert_eq!(response["totals"]["tabs"], 0);
    }

    #[test]
    fn empty_scope_still_prunes_cache_to_snapshot() {
        let path = cache_path("empty-scope-prunes");
        let mut cache = PageCache::default();
        cache.store_success(
            Some("test-profile"),
            1,
            "https://closed.example",
            None,
            "<p>Closed</p>",
            13,
            6,
            Some("complete"),
            false,
            false,
            1000,
        );
        cache.save_if_dirty(&path).unwrap();

        let mut orch = ReadMarkdownOrchestration::new(
            &serde_json::json!({"windowId": 999}),
            context_with_cache(path.clone()),
        );
        let _ = orch.start();
        let step = orch.step(snapshot_with(vec![
            serde_json::json!({"tabId": 2, "windowId": 100, "url": "https://open.example", "title": "Open", "groupId": -1}),
        ]));
        let OrchStep::Complete { response, .. } = step else {
            panic!("expected Complete")
        };
        assert_eq!(response["totals"]["tabs"], 0);
        let cache = PageCache::load(&path);
        assert!(cache
            .lookup_open_tab(Some("test-profile"), 1, "https://closed.example")
            .is_none());
    }

    #[test]
    fn read_markdown_single_tab() {
        let params = serde_json::json!({"extract": true, "maxChars": 10000});
        let mut orch = ReadMarkdownOrchestration::new(&params, OrchestrationContext::default());
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
        assert_eq!(params["expectedUrl"], "https://a.com");
        assert_eq!(params["maxChars"], Value::Null);

        let step = orch.step(html_response("<h1>Hello</h1><p>World.</p>", 12));
        let OrchStep::Complete { response, .. } = step else {
            panic!("expected Complete")
        };
        let entries = response["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["tabId"], 1);
        assert_eq!(entries[0]["status"], STATUS_READ);
        assert_eq!(entries[0]["cached"], false);
        assert_eq!(entries[0]["diagnostics"]["source"], "live");
        assert!(entries[0]["markdown"].as_str().unwrap().contains("Hello"));
        assert_eq!(entries[0]["truncated"], false);
    }

    #[test]
    fn live_read_writes_cache_and_live_provenance() {
        let path = cache_path("live-write");
        let params = serde_json::json!({});
        let mut orch = ReadMarkdownOrchestration::new(&params, context_with_cache(path.clone()));
        let _ = orch.start();
        let snap = snapshot_with(vec![
            serde_json::json!({"tabId": 1, "windowId": 100, "url": "https://a.com/page#frag", "title": "A", "groupId": -1}),
        ]);
        let _ = orch.step(snap);
        let step = orch.step(html_response("<h1>Cached</h1>", 6));
        let OrchStep::Complete { response, .. } = step else {
            panic!("expected Complete")
        };
        assert_eq!(response["entries"][0]["status"], STATUS_READ);
        assert_eq!(response["entries"][0]["cached"], false);
        assert_eq!(response["entries"][0]["diagnostics"]["source"], "live");
        assert!(path.exists());

        let cache = PageCache::load(&path);
        assert!(cache
            .lookup_open_tab(Some("test-profile"), 1, "https://a.com/page#frag")
            .is_some());
    }

    #[test]
    fn protected_failure_falls_back_to_cache() {
        let path = cache_path("protected-fallback");
        let mut cache = PageCache::default();
        cache.store_success(
            Some("test-profile"),
            1,
            "https://a.com",
            Some("A"),
            "<h1>Cached body</h1>",
            20,
            11,
            Some("complete"),
            false,
            false,
            1000,
        );
        cache.save_if_dirty(&path).unwrap();

        let mut orch =
            ReadMarkdownOrchestration::new(&serde_json::json!({}), context_with_cache(path));
        let _ = orch.start();
        let _ = orch.step(snapshot_with(vec![
            serde_json::json!({"tabId": 1, "windowId": 100, "url": "https://a.com", "title": "A", "groupId": -1}),
        ]));
        let step = orch.step(status_response(STATUS_PROTECTED));
        let OrchStep::Complete { response, .. } = step else {
            panic!("expected Complete")
        };
        let entry = &response["entries"][0];
        assert_eq!(entry["status"], STATUS_CACHED);
        assert_eq!(entry["cached"], true);
        assert_eq!(entry["error"], Value::Null);
        assert_eq!(entry["emptyReason"], Value::Null);
        assert_eq!(entry["diagnostics"]["source"], "cache");
        assert_eq!(entry["diagnostics"]["cacheMatch"], "exact");
        assert_eq!(entry["diagnostics"]["cachedAt"], 1000);
        assert_eq!(entry["diagnostics"]["sourceHtmlChars"], 20);
    }

    #[test]
    fn cached_fallback_reports_duplicate_exact_match() {
        let path = cache_path("duplicate-exact-fallback");
        let mut cache = PageCache::default();
        cache.store_success(
            Some("test-profile"),
            1,
            "https://a.com/page?q=1#frag",
            Some("A"),
            "<h1>Cached duplicate</h1>",
            25,
            16,
            Some("complete"),
            false,
            false,
            1000,
        );
        cache.save_if_dirty(&path).unwrap();

        let mut orch =
            ReadMarkdownOrchestration::new(&serde_json::json!({}), context_with_cache(path));
        let _ = orch.start();
        let _ = orch.step(snapshot_with(vec![
            serde_json::json!({"tabId": 2, "windowId": 100, "url": "https://a.com/page?q=1#frag", "title": "A", "groupId": -1}),
        ]));
        let step = orch.step(status_response(STATUS_PROTECTED));
        let OrchStep::Complete { response, .. } = step else {
            panic!("expected Complete")
        };
        let entry = &response["entries"][0];
        assert_eq!(entry["status"], STATUS_CACHED);
        assert_eq!(entry["diagnostics"]["cacheMatch"], "duplicateExact");
    }

    #[test]
    fn cached_fallback_reports_canonical_match() {
        let path = cache_path("canonical-fallback");
        let mut cache = PageCache::default();
        cache.store_success(
            Some("test-profile"),
            1,
            "https://a.com/page?q=1#old",
            Some("A"),
            "<h1>Cached canonical</h1>",
            25,
            16,
            Some("complete"),
            false,
            false,
            1000,
        );
        cache.save_if_dirty(&path).unwrap();

        let mut orch =
            ReadMarkdownOrchestration::new(&serde_json::json!({}), context_with_cache(path));
        let _ = orch.start();
        let _ = orch.step(snapshot_with(vec![
            serde_json::json!({"tabId": 1, "windowId": 100, "url": "https://a.com/page?q=1#new", "title": "A", "groupId": -1}),
        ]));
        let step = orch.step(status_response(STATUS_PROTECTED));
        let OrchStep::Complete { response, .. } = step else {
            panic!("expected Complete")
        };
        let entry = &response["entries"][0];
        assert_eq!(entry["status"], STATUS_CACHED);
        assert_eq!(entry["diagnostics"]["cacheMatch"], "canonical");
    }

    #[test]
    fn cached_fallback_respects_current_max_html_chars() {
        let path = cache_path("cached-max-html");
        let mut cache = PageCache::default();
        cache.store_success(
            Some("test-profile"),
            1,
            "https://a.com",
            Some("A"),
            "<h1>First</h1><p>Second</p>",
            27,
            11,
            Some("complete"),
            false,
            false,
            1000,
        );
        cache.save_if_dirty(&path).unwrap();

        let mut orch = ReadMarkdownOrchestration::new(
            &serde_json::json!({"maxHtmlChars": 14}),
            context_with_cache(path),
        );
        let _ = orch.start();
        let _ = orch.step(snapshot_with(vec![
            serde_json::json!({"tabId": 1, "windowId": 100, "url": "https://a.com", "title": "A", "groupId": -1}),
        ]));
        let step = orch.step(status_response(STATUS_PROTECTED));
        let OrchStep::Complete { response, .. } = step else {
            panic!("expected Complete")
        };
        let entry = &response["entries"][0];
        assert_eq!(entry["status"], STATUS_CACHED);
        assert_eq!(entry["diagnostics"]["truncatedHtml"], true);
        let markdown = entry["markdown"].as_str().unwrap();
        assert!(markdown.contains("First"));
        assert!(!markdown.contains("Second"));
    }

    #[test]
    fn timed_out_failure_falls_back_to_cache() {
        let path = cache_path("timeout-fallback");
        let mut cache = PageCache::default();
        cache.store_success(
            Some("test-profile"),
            1,
            "https://a.com",
            None,
            "<p>Stale ok</p>",
            15,
            8,
            Some("complete"),
            false,
            false,
            1000,
        );
        cache.save_if_dirty(&path).unwrap();

        let mut orch =
            ReadMarkdownOrchestration::new(&serde_json::json!({}), context_with_cache(path));
        let _ = orch.start();
        let _ = orch.step(snapshot_with(vec![
            serde_json::json!({"tabId": 1, "windowId": 100, "url": "https://a.com", "title": "A", "groupId": -1}),
        ]));
        let step = orch.step(status_response(STATUS_TIMED_OUT));
        let OrchStep::Complete { response, .. } = step else {
            panic!("expected Complete")
        };
        assert_eq!(response["entries"][0]["status"], STATUS_CACHED);
    }

    #[test]
    fn no_fallback_without_cache() {
        let mut orch =
            ReadMarkdownOrchestration::new(&serde_json::json!({}), OrchestrationContext::default());
        let _ = orch.start();
        let _ = orch.step(snapshot_with(vec![
            serde_json::json!({"tabId": 1, "windowId": 100, "url": "https://a.com", "title": "A", "groupId": -1}),
        ]));
        let step = orch.step(status_response(STATUS_PROTECTED));
        let OrchStep::Complete { response, .. } = step else {
            panic!("expected Complete")
        };
        assert_eq!(response["entries"][0]["status"], STATUS_PROTECTED);
        assert_eq!(response["entries"][0]["cached"], false);
    }

    #[test]
    fn url_mismatch_does_not_fallback_to_cache() {
        let path = cache_path("url-mismatch-no-fallback");
        let mut cache = PageCache::default();
        cache.store_success(
            Some("test-profile"),
            1,
            "https://a.com",
            None,
            "<p>Cached</p>",
            13,
            6,
            Some("complete"),
            false,
            false,
            1000,
        );
        cache.save_if_dirty(&path).unwrap();

        let mut orch =
            ReadMarkdownOrchestration::new(&serde_json::json!({}), context_with_cache(path));
        let _ = orch.start();
        let _ = orch.step(snapshot_with(vec![
            serde_json::json!({"tabId": 1, "windowId": 100, "url": "https://a.com", "title": "A", "groupId": -1}),
        ]));
        let step = orch.step(status_response("URL_MISMATCH"));
        let OrchStep::Complete { response, .. } = step else {
            panic!("expected Complete")
        };
        assert_eq!(response["entries"][0]["status"], STATUS_EXTRACTION_FAILED);
        assert_eq!(response["entries"][0]["cached"], false);
    }

    #[test]
    fn unsupported_url_does_not_fallback() {
        let path = cache_path("unsupported-no-fallback");
        let mut cache = PageCache::default();
        cache.store_success(
            Some("test-profile"),
            1,
            "chrome://settings",
            None,
            "<p>Never</p>",
            12,
            5,
            Some("complete"),
            false,
            false,
            1000,
        );
        cache.save_if_dirty(&path).unwrap();
        let mut orch =
            ReadMarkdownOrchestration::new(&serde_json::json!({}), context_with_cache(path));
        let _ = orch.start();
        let step = orch.step(snapshot_with(vec![
            serde_json::json!({"tabId": 1, "windowId": 100, "url": "chrome://settings", "title": "Settings", "groupId": -1}),
        ]));
        let OrchStep::Complete { response, .. } = step else {
            panic!("expected Complete")
        };
        assert_eq!(response["entries"][0]["status"], STATUS_UNSUPPORTED_URL);
        assert_eq!(response["entries"][0]["cached"], false);
        assert_eq!(response["totals"]["tasks"], 0);
    }

    #[test]
    fn incognito_neither_reads_nor_writes_cache() {
        let path = cache_path("incognito-no-write");
        let mut cache = PageCache::default();
        cache.store_success(
            Some("test-profile"),
            1,
            "https://a.com",
            None,
            "<h1>Cached secret</h1>",
            22,
            13,
            Some("complete"),
            false,
            false,
            1000,
        );
        cache.save_if_dirty(&path).unwrap();

        let mut orch = ReadMarkdownOrchestration::new(
            &serde_json::json!({}),
            context_with_cache(path.clone()),
        );
        let _ = orch.start();
        let _ = orch.step(snapshot_with(vec![
            serde_json::json!({"tabId": 1, "windowId": 100, "incognito": true, "url": "https://a.com", "title": "A", "groupId": -1}),
        ]));
        let step = orch.step(html_response("<h1>Secret</h1>", 6));
        let OrchStep::Complete { response, .. } = step else {
            panic!("expected Complete")
        };
        assert_eq!(response["entries"][0]["status"], STATUS_READ);
        assert_eq!(response["entries"][0]["cached"], false);
        let cache = PageCache::load(&path);
        assert!(cache
            .lookup_open_tab(Some("test-profile"), 1, "https://a.com")
            .is_none());
    }

    #[test]
    fn incognito_failure_does_not_fallback_to_cache() {
        let path = cache_path("incognito-no-fallback");
        let mut cache = PageCache::default();
        cache.store_success(
            Some("test-profile"),
            1,
            "https://a.com",
            None,
            "<h1>Cached secret</h1>",
            22,
            13,
            Some("complete"),
            false,
            false,
            1000,
        );
        cache.save_if_dirty(&path).unwrap();

        let mut orch =
            ReadMarkdownOrchestration::new(&serde_json::json!({}), context_with_cache(path));
        let _ = orch.start();
        let _ = orch.step(snapshot_with(vec![
            serde_json::json!({"tabId": 1, "windowId": 100, "incognito": true, "url": "https://a.com", "title": "A", "groupId": -1}),
        ]));
        let step = orch.step(status_response(STATUS_PROTECTED));
        let OrchStep::Complete { response, .. } = step else {
            panic!("expected Complete")
        };
        assert_eq!(response["entries"][0]["status"], STATUS_PROTECTED);
        assert_eq!(response["entries"][0]["cached"], false);
    }

    #[test]
    fn truncated_html_is_not_cached() {
        let path = cache_path("truncated-no-cache");
        let mut response = html_response("<h1>Cut</h1>", 3);
        response["truncatedHtml"] = true.into();
        let mut orch = ReadMarkdownOrchestration::new(
            &serde_json::json!({}),
            context_with_cache(path.clone()),
        );
        let _ = orch.start();
        let _ = orch.step(snapshot_with(vec![
            serde_json::json!({"tabId": 1, "windowId": 100, "url": "https://a.com", "title": "A", "groupId": -1}),
        ]));
        let _ = orch.step(response);
        let cache = PageCache::load(&path);
        assert!(cache
            .lookup_open_tab(Some("test-profile"), 1, "https://a.com")
            .is_none());
    }

    #[test]
    fn cache_is_pruned_to_open_current_urls() {
        let path = cache_path("prune-open");
        let mut cache = PageCache::default();
        cache.store_success(
            Some("test-profile"),
            1,
            "https://a.com/old",
            None,
            "<p>Old</p>",
            10,
            3,
            Some("complete"),
            false,
            false,
            1000,
        );
        cache.store_success(
            Some("test-profile"),
            2,
            "https://b.com",
            None,
            "<p>Keep</p>",
            11,
            4,
            Some("complete"),
            false,
            false,
            1001,
        );
        cache.save_if_dirty(&path).unwrap();

        let mut orch = ReadMarkdownOrchestration::new(
            &serde_json::json!({"tabIds": [2]}),
            context_with_cache(path.clone()),
        );
        let _ = orch.start();
        let _ = orch.step(snapshot_with(vec![
            serde_json::json!({"tabId": 1, "windowId": 100, "url": "https://a.com/new", "title": "A", "groupId": -1}),
            serde_json::json!({"tabId": 2, "windowId": 100, "url": "https://b.com", "title": "B", "groupId": -1}),
        ]));
        let _ = orch.step(status_response(STATUS_TIMED_OUT));

        let cache = PageCache::load(&path);
        assert!(cache
            .lookup_open_tab(Some("test-profile"), 1, "https://a.com/old")
            .is_none());
        assert!(cache
            .lookup_open_tab(Some("test-profile"), 2, "https://b.com")
            .is_some());
    }

    #[test]
    fn cache_is_saved_only_when_dirty_read_completes() {
        let path = cache_path("save-on-complete");
        let mut orch = ReadMarkdownOrchestration::new(
            &serde_json::json!({}),
            context_with_cache(path.clone()),
        );
        let _ = orch.start();
        let _ = orch.step(snapshot_with(vec![
            serde_json::json!({"tabId": 1, "windowId": 100, "url": "https://a.com", "title": "A", "groupId": -1}),
            serde_json::json!({"tabId": 2, "windowId": 100, "url": "https://b.com", "title": "B", "groupId": -1}),
        ]));

        let step = orch.step(html_response("<p>A</p>", 1));
        assert!(matches!(step, OrchStep::SendPrimitive { .. }));
        assert!(
            !path.exists(),
            "cache should not be persisted until all readTabs work completes"
        );

        let step = orch.step(html_response("<p>B</p>", 1));
        assert!(matches!(step, OrchStep::Complete { .. }));
        assert!(
            path.exists(),
            "dirty cache should be persisted on completion"
        );
        assert!(PageCache::load(&path)
            .lookup_open_tab(Some("test-profile"), 1, "https://a.com")
            .is_some());
        assert!(PageCache::load(&path)
            .lookup_open_tab(Some("test-profile"), 2, "https://b.com")
            .is_some());
    }

    #[test]
    fn cache_save_failure_does_not_fail_live_read() {
        let path = cache_path("save-failure-nonfatal");
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, b"not-a-directory").unwrap();
        let mut orch =
            ReadMarkdownOrchestration::new(&serde_json::json!({}), context_with_cache(path));
        let _ = orch.start();
        let _ = orch.step(snapshot_with(vec![
            serde_json::json!({"tabId": 1, "windowId": 100, "url": "https://a.com", "title": "A", "groupId": -1}),
        ]));

        let step = orch.step(html_response("<h1>Hello</h1>", 5));
        let OrchStep::Complete { response, .. } = step else {
            panic!("expected Complete despite cache save failure")
        };
        assert_eq!(response["entries"][0]["status"], STATUS_READ);
        assert_eq!(response["entries"][0]["cached"], false);
    }

    #[test]
    fn read_markdown_multiple_tabs_preserves_order() {
        let params = serde_json::json!({});
        let mut orch = ReadMarkdownOrchestration::new(&params, OrchestrationContext::default());
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
        let mut orch = ReadMarkdownOrchestration::new(&params, OrchestrationContext::default());
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
        let mut orch = ReadMarkdownOrchestration::new(&params, OrchestrationContext::default());
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
        let mut orch = ReadMarkdownOrchestration::new(&params, OrchestrationContext::default());
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
        let mut orch = ReadMarkdownOrchestration::new(&params, OrchestrationContext::default());
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
