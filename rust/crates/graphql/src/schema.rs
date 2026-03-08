use juniper::{graphql_object, EmptySubscription, FieldResult, GraphQLEnum, RootNode};

use crate::context::GqlContext;
use crate::convert::{tab_from_value, windows_from_snapshot};
use crate::types::*;

pub(crate) type Schema = RootNode<'static, Query, Mutation, EmptySubscription<GqlContext>>;

pub(crate) fn create_schema() -> Schema {
    Schema::new(Query, Mutation, EmptySubscription::new())
}

/// Sort order for tab queries.
#[derive(Debug, Clone, Copy, GraphQLEnum)]
pub(crate) enum TabOrderBy {
    /// Most recently accessed first.
    LastAccessedDesc,
    /// Least recently accessed first.
    LastAccessedAsc,
    /// Alphabetical by title (A-Z).
    TitleAsc,
    /// Reverse alphabetical by title (Z-A).
    TitleDesc,
    /// Tab position within window (default).
    IndexAsc,
}

/// Query root — read-only access to tab data from a snapshot.
const DEFAULT_LIMIT: i32 = 20;

pub(crate) struct Query;

#[graphql_object(context = GqlContext)]
impl Query {
    /// All windows with their tabs and groups.
    fn windows(ctx: &GqlContext) -> Vec<Window> {
        windows_from_snapshot(&ctx.snapshot)
    }

    /// A single window by ID.
    fn window(
        ctx: &GqlContext,
        #[graphql(description = "Chrome window identifier.")] id: i32,
    ) -> Option<Window> {
        windows_from_snapshot(&ctx.snapshot)
            .into_iter()
            .find(|w| w.window_id == id)
    }

    /// Paginated tabs with optional scope filters.
    #[allow(clippy::too_many_arguments)]
    fn tabs(
        ctx: &GqlContext,
        #[graphql(description = "Restrict to tabs in this window.")] window_id: Option<i32>,
        #[graphql(description = "Restrict to tabs in this group (by group ID).")] group_id: Option<
            i32,
        >,
        #[graphql(description = "Restrict to tabs whose group has this title.")]
        group_title: Option<String>,
        #[graphql(description = "When true, return only ungrouped tabs (groupId = -1).")]
        ungrouped: Option<bool>,
        #[graphql(description = "Sort order (default: INDEX_ASC).")] order_by: Option<TabOrderBy>,
        #[graphql(description = "Maximum number of tabs to return per page (default 20).")]
        limit: Option<i32>,
        #[graphql(description = "Zero-based offset into the result set.")] offset: Option<i32>,
    ) -> TabPage {
        let windows = windows_from_snapshot(&ctx.snapshot);
        let mut tabs: Vec<Tab> = windows.into_iter().flat_map(|w| w.tabs).collect();

        if let Some(wid) = window_id {
            tabs.retain(|t| t.window_id == wid);
        }
        if let Some(gid) = group_id {
            tabs.retain(|t| t.group_id == gid);
        }
        if let Some(ref title) = group_title {
            tabs.retain(|t| t.group_title.as_deref() == Some(title.as_str()));
        }
        if ungrouped == Some(true) {
            tabs.retain(|t| t.group_id == -1);
        }

        match order_by.unwrap_or(TabOrderBy::IndexAsc) {
            TabOrderBy::LastAccessedDesc => {
                tabs.sort_by(|a, b| {
                    let ta = a.last_accessed_at.unwrap_or(0.0);
                    let tb = b.last_accessed_at.unwrap_or(0.0);
                    tb.partial_cmp(&ta).unwrap_or(std::cmp::Ordering::Equal)
                });
            }
            TabOrderBy::LastAccessedAsc => {
                tabs.sort_by(|a, b| {
                    let ta = a.last_accessed_at.unwrap_or(0.0);
                    let tb = b.last_accessed_at.unwrap_or(0.0);
                    ta.partial_cmp(&tb).unwrap_or(std::cmp::Ordering::Equal)
                });
            }
            TabOrderBy::TitleAsc => {
                tabs.sort_by(|a, b| a.title.to_lowercase().cmp(&b.title.to_lowercase()));
            }
            TabOrderBy::TitleDesc => {
                tabs.sort_by(|a, b| b.title.to_lowercase().cmp(&a.title.to_lowercase()));
            }
            TabOrderBy::IndexAsc => {} // default order from snapshot
        }

        paginate_tabs(tabs, limit, offset)
    }

    /// A single tab by ID.
    fn tab(
        ctx: &GqlContext,
        #[graphql(description = "Chrome tab identifier.")] id: i32,
    ) -> Option<Tab> {
        windows_from_snapshot(&ctx.snapshot)
            .into_iter()
            .flat_map(|w| w.tabs)
            .find(|t| t.tab_id == id)
    }

    /// Groups filtered by optional window ID.
    fn groups(
        ctx: &GqlContext,
        #[graphql(description = "Restrict to groups in this window.")] window_id: Option<i32>,
    ) -> Vec<Group> {
        let windows = windows_from_snapshot(&ctx.snapshot);
        windows
            .into_iter()
            .filter(|w| window_id.is_none() || Some(w.window_id) == window_id)
            .flat_map(|w| w.groups)
            .collect()
    }

    /// Health check — verifies the host is reachable.
    fn ping(ctx: &GqlContext) -> FieldResult<PingResult> {
        let start = std::time::Instant::now();
        let result = ctx.sender.send("ping", serde_json::json!({}));
        let latency_ms = start.elapsed().as_secs_f64() * 1000.0;
        match result {
            Ok(_) => Ok(PingResult {
                ok: true,
                latency_ms,
            }),
            Err(e) => Err(juniper::FieldError::new(e, juniper::Value::Null)),
        }
    }

    /// Analyze tabs for staleness and duplicates.
    fn analyze(
        ctx: &GqlContext,
        #[graphql(
            description = "Number of days since last focus to consider a tab stale (default 30)."
        )]
        stale_days: Option<i32>,
        #[graphql(description = "Restrict analysis to this window.")] window_id: Option<i32>,
        #[graphql(description = "Restrict analysis to this group (by group ID).")] group_id: Option<
            i32,
        >,
        #[graphql(description = "Restrict analysis to tabs whose group has this title.")]
        group_title: Option<String>,
        #[graphql(description = "When true, analyze only ungrouped tabs (groupId = -1).")]
        ungrouped: Option<bool>,
    ) -> FieldResult<AnalyzeResult> {
        let mut params = serde_json::Map::new();
        if let Some(days) = stale_days {
            params.insert("staleDays".to_string(), serde_json::json!(days));
        }
        if let Some(wid) = window_id {
            params.insert("windowId".to_string(), serde_json::json!(wid as i64));
        }
        if let Some(gid) = group_id {
            params.insert("groupId".to_string(), serde_json::json!(gid as i64));
        }
        if let Some(ref title) = group_title {
            params.insert("groupTitle".to_string(), serde_json::json!(title));
        }
        if ungrouped == Some(true) {
            params.insert("ungrouped".to_string(), serde_json::json!(true));
        }
        let response = ctx
            .sender
            .send("analyze", serde_json::Value::Object(params))
            .map_err(|e| juniper::FieldError::new(e, juniper::Value::Null))?;

        let stale_tabs = response
            .get("stale")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0) as i32;
        let duplicate_tabs = response
            .get("duplicates")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0) as i32;
        let total_tabs = response
            .get("summary")
            .and_then(|s| s.get("totalTabs"))
            .or_else(|| response.get("totalTabs"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32;
        let raw = serde_json::to_string(&response).unwrap_or_default();

        Ok(AnalyzeResult {
            stale_tabs,
            duplicate_tabs,
            total_tabs,
            raw,
        })
    }

    /// Recent undo history entries.
    fn history(
        ctx: &GqlContext,
        #[graphql(description = "Maximum number of entries to return (default 20).")] limit: Option<
            i32,
        >,
    ) -> FieldResult<Vec<HistoryEntry>> {
        let mut params = serde_json::Map::new();
        if let Some(lim) = limit {
            params.insert("limit".to_string(), serde_json::json!(lim));
        }
        let response = ctx
            .sender
            .send("history", serde_json::Value::Object(params))
            .map_err(|e| juniper::FieldError::new(e, juniper::Value::Null))?;

        let entries = response
            .as_array()
            .unwrap_or(&Vec::new())
            .iter()
            .filter_map(|entry| {
                Some(HistoryEntry {
                    txid: entry.get("txid").and_then(|v| v.as_str())?.to_string(),
                    action: entry
                        .get("action")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    summary: entry
                        .get("summary")
                        .map(|v| v.to_string())
                        .unwrap_or_default(),
                    created_at: entry
                        .get("createdAt")
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0),
                })
            })
            .collect();

        Ok(entries)
    }

    /// Persisted browser-state checkpoints captured from live browser events and startup syncs.
    fn browser_state_history(
        ctx: &GqlContext,
        #[graphql(description = "Maximum number of persisted checkpoints to return (default 20).")]
        limit: Option<i32>,
    ) -> FieldResult<Vec<BrowserStateHistoryEntry>> {
        let mut params = serde_json::Map::new();
        if let Some(lim) = limit {
            params.insert("limit".to_string(), serde_json::json!(lim));
        }
        let response = ctx
            .sender
            .send("browser-state-history", serde_json::Value::Object(params))
            .map_err(|e| juniper::FieldError::new(e, juniper::Value::Null))?;
        let entries = response
            .as_array()
            .unwrap_or(&Vec::new())
            .iter()
            .filter_map(browser_state_history_from_value)
            .collect();
        Ok(entries)
    }

    /// The most recently persisted browser-state snapshot.
    fn latest_browser_state(ctx: &GqlContext) -> FieldResult<Option<BrowserStateSnapshot>> {
        let response = ctx
            .sender
            .send("browser-state-latest", serde_json::json!({}))
            .map_err(|e| juniper::FieldError::new(e, juniper::Value::Null))?;
        if response.is_null() {
            return Ok(None);
        }
        Ok(browser_state_snapshot_from_value(&response))
    }

    /// Raw browser-observed event history captured into the persisted store.
    fn browser_state_events(
        ctx: &GqlContext,
        #[graphql(description = "Maximum number of events to return (default 50).")] limit: Option<
            i32,
        >,
        #[graphql(description = "Optional event kind filter, e.g. tabs.onUpdated.")] kind: Option<
            String,
        >,
    ) -> FieldResult<Vec<BrowserStateEvent>> {
        let mut params = serde_json::Map::new();
        if let Some(lim) = limit {
            params.insert("limit".to_string(), serde_json::json!(lim));
        }
        if let Some(kind) = kind {
            params.insert("kind".to_string(), serde_json::json!(kind));
        }
        let response = ctx
            .sender
            .send("browser-state-events", serde_json::Value::Object(params))
            .map_err(|e| juniper::FieldError::new(e, juniper::Value::Null))?;
        let entries = response
            .as_array()
            .unwrap_or(&Vec::new())
            .iter()
            .filter_map(browser_state_event_from_value)
            .collect();
        Ok(entries)
    }

    /// Group history with stable logical identity metadata for future restore tooling.
    fn browser_state_group_history(
        ctx: &GqlContext,
        #[graphql(description = "Maximum number of group-history entries to return (default 50).")]
        limit: Option<i32>,
        #[graphql(description = "Optional filter by current or historical group title.")]
        title: Option<String>,
        #[graphql(description = "Optional filter by logical group ID.")] logical_group_id: Option<
            String,
        >,
    ) -> FieldResult<Vec<BrowserStateGroupEntry>> {
        let mut params = serde_json::Map::new();
        if let Some(lim) = limit {
            params.insert("limit".to_string(), serde_json::json!(lim));
        }
        if let Some(title) = title {
            params.insert("title".to_string(), serde_json::json!(title));
        }
        if let Some(logical_group_id) = logical_group_id {
            params.insert(
                "logicalGroupId".to_string(),
                serde_json::json!(logical_group_id),
            );
        }
        let response = ctx
            .sender
            .send(
                "browser-state-group-history",
                serde_json::Value::Object(params),
            )
            .map_err(|e| juniper::FieldError::new(e, juniper::Value::Null))?;
        let entries = response
            .as_array()
            .unwrap_or(&Vec::new())
            .iter()
            .filter_map(browser_state_group_from_value)
            .collect();
        Ok(entries)
    }

    /// Inspect tabs with page-meta and selector signals.
    #[allow(clippy::too_many_arguments)]
    fn inspect_tabs(
        ctx: &GqlContext,
        #[graphql(description = "Restrict inspection to this window.")] window_id: Option<i32>,
        #[graphql(description = "Restrict inspection to this group (by group ID).")]
        group_id: Option<i32>,
        #[graphql(description = "Restrict inspection to tabs whose group has this title.")]
        group_title: Option<String>,
        #[graphql(description = "When true, inspect only ungrouped tabs (groupId = -1).")]
        ungrouped: Option<bool>,
        #[graphql(description = "Restrict inspection to these specific tab IDs.")] tab_ids: Option<
            Vec<i32>,
        >,
        #[graphql(description = "Signal IDs to run (e.g. page-meta, selector).")] signals: Option<
            Vec<String>,
        >,
        #[graphql(description = "Selector specs for selector-based extraction.")] selectors: Option<
            Vec<SelectorSpecInput>,
        >,
        #[graphql(description = "Override timeout (ms) for each signal.")]
        signal_timeout_ms: Option<i32>,
        #[graphql(description = "Wait mode (load, dom, settle, none).")] wait_for: Option<String>,
        #[graphql(description = "Wait timeout in ms.")] wait_timeout_ms: Option<i32>,
    ) -> FieldResult<InspectResult> {
        let mut params = scoped_params(window_id, group_id, group_title, ungrouped, tab_ids);
        if let Some(mut requested_signals) = signals {
            if selectors.as_ref().is_some_and(|s| !s.is_empty())
                && !requested_signals.iter().any(|s| s == "selector")
            {
                requested_signals.push("selector".to_string());
            }
            params.insert("signals".to_string(), serde_json::json!(requested_signals));
        }
        if let Some(selector_specs) = selectors {
            let values: Vec<serde_json::Value> = selector_specs
                .into_iter()
                .map(|selector| {
                    serde_json::json!({
                        "name": selector.name,
                        "selector": selector.selector,
                        "attr": selector.attr,
                    })
                })
                .collect();
            if !values.is_empty() {
                params.insert(
                    "selectorSpecs".to_string(),
                    serde_json::Value::Array(values),
                );
            }
        }
        if let Some(timeout) = signal_timeout_ms {
            params.insert("signalTimeoutMs".to_string(), serde_json::json!(timeout));
        }
        if let Some(ref wait) = wait_for {
            params.insert("waitFor".to_string(), serde_json::json!(wait));
        }
        if let Some(timeout) = wait_timeout_ms {
            params.insert("waitTimeoutMs".to_string(), serde_json::json!(timeout));
        }

        let response = ctx
            .sender
            .send("inspect", serde_json::Value::Object(params))
            .map_err(|e| juniper::FieldError::new(e, juniper::Value::Null))?;

        Ok(parse_inspect_result(&response))
    }

    /// Build a report of tab metadata and extracted descriptions.
    fn report_tabs(
        ctx: &GqlContext,
        #[graphql(description = "Restrict reporting to this window.")] window_id: Option<i32>,
        #[graphql(description = "Restrict reporting to this group (by group ID).")]
        group_id: Option<i32>,
        #[graphql(description = "Restrict reporting to tabs whose group has this title.")]
        group_title: Option<String>,
        #[graphql(description = "When true, report only ungrouped tabs (groupId = -1).")]
        ungrouped: Option<bool>,
        #[graphql(description = "Restrict reporting to these specific tab IDs.")] tab_ids: Option<
            Vec<i32>,
        >,
    ) -> FieldResult<ReportResult> {
        let params = scoped_params(window_id, group_id, group_title, ungrouped, tab_ids);
        let response = ctx
            .sender
            .send("report", serde_json::Value::Object(params))
            .map_err(|e| juniper::FieldError::new(e, juniper::Value::Null))?;

        Ok(parse_report_result(&response))
    }

    /// Capture screenshots for matching tabs.
    #[allow(clippy::too_many_arguments)]
    fn capture_screenshots(
        ctx: &GqlContext,
        #[graphql(description = "Restrict capture to this window.")] window_id: Option<i32>,
        #[graphql(description = "Restrict capture to this group (by group ID).")] group_id: Option<
            i32,
        >,
        #[graphql(description = "Restrict capture to tabs whose group has this title.")]
        group_title: Option<String>,
        #[graphql(description = "When true, capture only ungrouped tabs (groupId = -1).")]
        ungrouped: Option<bool>,
        #[graphql(description = "Restrict capture to these specific tab IDs.")] tab_ids: Option<
            Vec<i32>,
        >,
        #[graphql(description = "Capture mode (viewport or full).")] mode: Option<String>,
        #[graphql(description = "Image format (png or jpeg).")] format: Option<String>,
        #[graphql(description = "JPEG quality (1-100).")] quality: Option<i32>,
        #[graphql(description = "Maximum tile dimension in pixels.")] tile_max_dim: Option<i32>,
        #[graphql(description = "Maximum bytes per tile.")] max_bytes: Option<i32>,
        #[graphql(description = "Wait mode (load, dom, settle, none).")] wait_for: Option<String>,
        #[graphql(description = "Wait timeout in ms.")] wait_timeout_ms: Option<i32>,
    ) -> FieldResult<ScreenshotResult> {
        let mut params = scoped_params(window_id, group_id, group_title, ungrouped, tab_ids);
        if let Some(ref value) = mode {
            params.insert("mode".to_string(), serde_json::json!(value));
        }
        if let Some(ref value) = format {
            params.insert("format".to_string(), serde_json::json!(value));
        }
        if let Some(value) = quality {
            params.insert("quality".to_string(), serde_json::json!(value));
        }
        if let Some(value) = tile_max_dim {
            params.insert("tileMaxDim".to_string(), serde_json::json!(value));
        }
        if let Some(value) = max_bytes {
            params.insert("maxBytes".to_string(), serde_json::json!(value));
        }
        if let Some(ref value) = wait_for {
            params.insert("waitFor".to_string(), serde_json::json!(value));
        }
        if let Some(value) = wait_timeout_ms {
            params.insert("waitTimeoutMs".to_string(), serde_json::json!(value));
        }

        let response = ctx
            .sender
            .send("screenshot", serde_json::Value::Object(params))
            .map_err(|e| juniper::FieldError::new(e, juniper::Value::Null))?;

        Ok(parse_screenshot_result(&response))
    }
}

fn paginate_tabs(tabs: Vec<Tab>, limit: Option<i32>, offset: Option<i32>) -> TabPage {
    let total = tabs.len() as i32;
    let off = offset.unwrap_or(0).max(0) as usize;
    let lim = limit.unwrap_or(DEFAULT_LIMIT).max(0) as usize;

    let page: Vec<Tab> = tabs.into_iter().skip(off).take(lim).collect();
    let has_more = (off + page.len()) < total as usize;

    TabPage {
        items: page,
        total,
        offset: off as i32,
        has_more,
    }
}

fn browser_state_history_from_value(value: &serde_json::Value) -> Option<BrowserStateHistoryEntry> {
    Some(BrowserStateHistoryEntry {
        snapshot_id: value.get("snapshotId").and_then(|v| v.as_i64())? as i32,
        recorded_at: value
            .get("recordedAt")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
        reason: value
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        event_count: value
            .get("eventCount")
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32,
        event_kinds: value
            .get("eventKinds")
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        previous_snapshot_id: value
            .get("previousSnapshotId")
            .and_then(|v| v.as_i64())
            .map(|v| v as i32),
        window_count: value
            .get("windowCount")
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32,
        group_count: value
            .get("groupCount")
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32,
        tab_count: value.get("tabCount").and_then(|v| v.as_i64()).unwrap_or(0) as i32,
    })
}

fn browser_state_event_from_value(value: &serde_json::Value) -> Option<BrowserStateEvent> {
    Some(BrowserStateEvent {
        event_id: value.get("eventId").and_then(|v| v.as_i64())? as i32,
        recorded_at: value
            .get("recordedAt")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
        reason: value
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        before_snapshot_id: value
            .get("beforeSnapshotId")
            .and_then(|v| v.as_i64())
            .map(|v| v as i32),
        after_snapshot_id: value
            .get("afterSnapshotId")
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32,
        kind: value
            .get("kind")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        browser_window_id: value
            .get("browserWindowId")
            .and_then(|v| v.as_i64())
            .map(|v| v as i32),
        browser_group_id: value
            .get("browserGroupId")
            .and_then(|v| v.as_i64())
            .map(|v| v as i32),
        browser_tab_id: value
            .get("browserTabId")
            .and_then(|v| v.as_i64())
            .map(|v| v as i32),
        payload_json: value
            .get("payloadJson")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
    })
}

fn browser_state_group_from_value(value: &serde_json::Value) -> Option<BrowserStateGroupEntry> {
    Some(BrowserStateGroupEntry {
        logical_group_id: value
            .get("logicalGroupId")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        logical_window_id: value
            .get("logicalWindowId")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        browser_group_id: value.get("browserGroupId").and_then(|v| v.as_i64())? as i32,
        browser_window_id: value.get("browserWindowId").and_then(|v| v.as_i64())? as i32,
        window_ordinal: value
            .get("windowOrdinal")
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32,
        title: value
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        color: value
            .get("color")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        collapsed: value.get("collapsed").and_then(|v| v.as_bool()),
        tab_count: value.get("tabCount").and_then(|v| v.as_i64()).unwrap_or(0) as i32,
        tab_urls: value
            .get("tabUrls")
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        snapshot_id: value
            .get("snapshotId")
            .and_then(|v| v.as_i64())
            .map(|v| v as i32),
        recorded_at: value.get("recordedAt").and_then(|v| v.as_f64()),
        reason: value
            .get("reason")
            .and_then(|v| v.as_str())
            .map(String::from),
    })
}

fn browser_state_snapshot_from_value(value: &serde_json::Value) -> Option<BrowserStateSnapshot> {
    let snapshot = value.get("snapshot")?;
    Some(BrowserStateSnapshot {
        snapshot_id: value.get("snapshotId").and_then(|v| v.as_i64())? as i32,
        recorded_at: value
            .get("recordedAt")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
        reason: value
            .get("reason")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        event_count: value
            .get("eventCount")
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32,
        event_kinds: value
            .get("eventKinds")
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| item.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default(),
        previous_snapshot_id: value
            .get("previousSnapshotId")
            .and_then(|v| v.as_i64())
            .map(|v| v as i32),
        windows: windows_from_snapshot(snapshot),
        groups: value
            .get("groups")
            .and_then(|v| v.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(browser_state_group_from_value)
                    .collect()
            })
            .unwrap_or_default(),
    })
}

fn duplicate_candidate_tabs(response: &serde_json::Value) -> Vec<Tab> {
    let mut result = Vec::new();
    let Some(groups) = response.get("duplicates").and_then(|v| v.as_array()) else {
        return result;
    };

    for group in groups {
        let Some(tabs) = group.get("tabs").and_then(|v| v.as_array()) else {
            continue;
        };
        for tab in tabs.iter().skip(1) {
            let window_id = tab.get("windowId").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
            if let Some(parsed) = tab_from_value(tab, window_id) {
                result.push(parsed);
            }
        }
    }

    result
}

fn scoped_params(
    window_id: Option<i32>,
    group_id: Option<i32>,
    group_title: Option<String>,
    ungrouped: Option<bool>,
    tab_ids: Option<Vec<i32>>,
) -> serde_json::Map<String, serde_json::Value> {
    let mut params = serde_json::Map::new();
    if let Some(wid) = window_id {
        params.insert("windowId".to_string(), serde_json::json!(wid as i64));
    }
    if let Some(gid) = group_id {
        params.insert("groupId".to_string(), serde_json::json!(gid as i64));
    }
    if let Some(title) = group_title {
        params.insert("groupTitle".to_string(), serde_json::json!(title));
    }
    if ungrouped == Some(true) {
        params.insert("ungrouped".to_string(), serde_json::json!(true));
    }
    if let Some(ids) = tab_ids {
        params.insert(
            "tabIds".to_string(),
            serde_json::json!(ids.iter().map(|&id| id as i64).collect::<Vec<_>>()),
        );
    }
    params
}

fn parse_inspect_result(response: &serde_json::Value) -> InspectResult {
    let totals = InspectTotals {
        tabs: response
            .get("totals")
            .and_then(|t| t.get("tabs"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32,
        signals: response
            .get("totals")
            .and_then(|t| t.get("signals"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32,
        tasks: response
            .get("totals")
            .and_then(|t| t.get("tasks"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32,
    };

    let entries = response
        .get("entries")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .map(|entry| {
                    let signals = entry
                        .get("signals")
                        .and_then(|v| v.as_object())
                        .map(|obj| {
                            obj.iter()
                                .map(|(name, value)| InspectSignalResult {
                                    name: name.clone(),
                                    value_json: serde_json::to_string(value).unwrap_or_default(),
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    InspectEntry {
                        tab_id: entry.get("tabId").and_then(|v| v.as_i64()).unwrap_or(0) as i32,
                        window_id: entry.get("windowId").and_then(|v| v.as_i64()).unwrap_or(0)
                            as i32,
                        url: entry
                            .get("url")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        title: entry
                            .get("title")
                            .and_then(|v| v.as_str())
                            .map(|v| v.to_string()),
                        signals,
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    InspectResult { totals, entries }
}

fn parse_report_result(response: &serde_json::Value) -> ReportResult {
    let entries = response
        .get("entries")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .map(|entry| ReportEntry {
                    tab_id: entry.get("tabId").and_then(|v| v.as_i64()).unwrap_or(0) as i32,
                    window_id: entry.get("windowId").and_then(|v| v.as_i64()).unwrap_or(0) as i32,
                    url: entry
                        .get("url")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    title: entry
                        .get("title")
                        .and_then(|v| v.as_str())
                        .map(|v| v.to_string()),
                    group_id: entry.get("groupId").and_then(|v| v.as_i64()).unwrap_or(-1) as i32,
                    group_title: entry
                        .get("groupTitle")
                        .and_then(|v| v.as_str())
                        .map(|v| v.to_string()),
                    group_color: entry
                        .get("groupColor")
                        .and_then(|v| v.as_str())
                        .map(|v| v.to_string()),
                    description: entry
                        .get("description")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    last_accessed_at: entry.get("lastAccessedAt").and_then(|v| v.as_f64()),
                })
                .collect()
        })
        .unwrap_or_default();

    ReportResult {
        generated_at: response
            .get("generatedAt")
            .and_then(|v| v.as_f64())
            .or_else(|| {
                response
                    .get("generatedAt")
                    .and_then(|v| v.as_i64())
                    .map(|v| v as f64)
            })
            .unwrap_or(0.0),
        totals: ReportTotals {
            tabs: response
                .get("totals")
                .and_then(|t| t.get("tabs"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as i32,
        },
        entries,
    }
}

fn parse_screenshot_result(response: &serde_json::Value) -> ScreenshotResult {
    let entries = response
        .get("entries")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .map(|entry| {
                    let tiles = entry
                        .get("tiles")
                        .and_then(|v| v.as_array())
                        .map(|tiles| {
                            tiles
                                .iter()
                                .map(|tile| ScreenshotTile {
                                    index: tile.get("index").and_then(|v| v.as_i64()).unwrap_or(0)
                                        as i32,
                                    total: tile
                                        .get("total")
                                        .and_then(|v| v.as_i64())
                                        .map(|v| v as i32),
                                    x: tile.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32,
                                    y: tile.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32,
                                    width: tile
                                        .get("width")
                                        .or_else(|| tile.get("w"))
                                        .and_then(|v| v.as_i64())
                                        .unwrap_or(0)
                                        as i32,
                                    height: tile
                                        .get("height")
                                        .or_else(|| tile.get("h"))
                                        .and_then(|v| v.as_i64())
                                        .unwrap_or(0)
                                        as i32,
                                    scale: tile
                                        .get("scale")
                                        .and_then(|v| v.as_f64())
                                        .unwrap_or(1.0),
                                    bytes: tile
                                        .get("bytes")
                                        .and_then(|v| v.as_i64())
                                        .map(|v| v as i32),
                                    scaled: tile.get("scaled").and_then(|v| v.as_bool()),
                                    oversized: tile.get("oversized").and_then(|v| v.as_bool()),
                                    data_url: tile
                                        .get("dataUrl")
                                        .or_else(|| tile.get("data"))
                                        .and_then(|v| v.as_str())
                                        .map(|v| v.to_string()),
                                })
                                .collect()
                        })
                        .unwrap_or_default();

                    ScreenshotEntry {
                        tab_id: entry.get("tabId").and_then(|v| v.as_i64()).unwrap_or(0) as i32,
                        window_id: entry.get("windowId").and_then(|v| v.as_i64()).unwrap_or(0)
                            as i32,
                        group_id: entry.get("groupId").and_then(|v| v.as_i64()).unwrap_or(-1)
                            as i32,
                        url: entry
                            .get("url")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        title: entry
                            .get("title")
                            .and_then(|v| v.as_str())
                            .map(|v| v.to_string()),
                        error: entry
                            .get("error")
                            .and_then(|v| v.get("message"))
                            .and_then(|v| v.as_str())
                            .map(|message| ScreenshotError {
                                message: message.to_string(),
                            }),
                        tiles,
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    ScreenshotResult {
        totals: ScreenshotTotals {
            tabs: response
                .get("totals")
                .and_then(|t| t.get("tabs"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as i32,
            tiles: response
                .get("totals")
                .and_then(|t| t.get("tiles"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as i32,
        },
        entries,
    }
}

/// Mutation root — tab actions with result projection.
pub(crate) struct Mutation;

#[graphql_object(context = GqlContext)]
impl Mutation {
    /// Close tabs by ID. Returns the transaction ID and remaining tabs.
    fn close_tabs(
        ctx: &GqlContext,
        #[graphql(description = "IDs of the tabs to close.")] tab_ids: Vec<i32>,
        #[graphql(
            description = "Must be true to actually close; omit or false for a dry-run preview."
        )]
        confirm: Option<bool>,
        #[graphql(description = "When true, return what would be closed without closing.")]
        dry_run: Option<bool>,
    ) -> FieldResult<CloseResult> {
        let mut params = serde_json::Map::new();
        params.insert(
            "tabIds".to_string(),
            serde_json::json!(tab_ids.iter().map(|&id| id as i64).collect::<Vec<_>>()),
        );
        if confirm == Some(true) {
            params.insert("confirmed".to_string(), serde_json::json!(true));
        }
        if dry_run == Some(true) {
            params.insert("dryRun".to_string(), serde_json::json!(true));
        }

        let response = ctx
            .sender
            .send("close", serde_json::Value::Object(params))
            .map_err(|e| juniper::FieldError::new(e, juniper::Value::Null))?;

        let txid = response
            .get("txid")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let closed_tabs = response
            .get("summary")
            .and_then(|s| s.get("closedTabs"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32;

        // Re-snapshot for remaining tabs
        let remaining_tabs = match ctx.sender.snapshot() {
            Ok(snap) => windows_from_snapshot(&snap)
                .into_iter()
                .flat_map(|w| w.tabs)
                .collect(),
            Err(e) => {
                return Err(juniper::FieldError::new(
                    format!("Tabs closed (txid: {txid}) but post-mutation snapshot failed: {e}"),
                    juniper::Value::Null,
                ));
            }
        };

        Ok(CloseResult {
            txid,
            closed_tabs,
            remaining_tabs,
        })
    }

    /// Open new tabs. Returns the created tabs.
    fn open_tabs(
        ctx: &GqlContext,
        #[graphql(description = "URLs to open.")] urls: Vec<String>,
        #[graphql(description = "Window to open tabs in; omit for the focused window.")]
        window_id: Option<i32>,
        #[graphql(description = "Group title to assign; creates the group if it doesn't exist.")]
        group: Option<String>,
        #[graphql(description = "Create a new window for the opened tabs.")] new_window: Option<
            bool,
        >,
    ) -> FieldResult<OpenResult> {
        let mut params = serde_json::Map::new();
        params.insert("urls".to_string(), serde_json::json!(urls));
        if let Some(wid) = window_id {
            params.insert("windowId".to_string(), serde_json::json!(wid as i64));
        }
        if let Some(ref g) = group {
            params.insert("groupTitle".to_string(), serde_json::json!(g));
        }
        if new_window == Some(true) {
            params.insert("newWindow".to_string(), serde_json::json!(true));
        }

        let response = ctx
            .sender
            .send("open", serde_json::Value::Object(params))
            .map_err(|e| juniper::FieldError::new(e, juniper::Value::Null))?;

        let tabs = response
            .get("created")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|t| {
                        let wid = t.get("windowId").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                        tab_from_value(t, wid)
                    })
                    .collect()
            })
            .unwrap_or_default();

        let skipped_urls = response
            .get("skipped")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .map(|s| SkippedUrl {
                        url: s
                            .get("url")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        reason: s
                            .get("reason")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                    })
                    .collect()
            })
            .unwrap_or_default();

        let window_id = response
            .get("windowId")
            .and_then(|v| v.as_i64())
            .map(|v| v as i32);

        let group_id = response
            .get("groupId")
            .and_then(|v| v.as_i64())
            .map(|v| v as i32);

        Ok(OpenResult {
            tabs,
            skipped_urls,
            window_id,
            group_id,
        })
    }

    /// Refresh (reload) tabs by ID.
    fn refresh_tabs(
        ctx: &GqlContext,
        #[graphql(description = "IDs of the tabs to reload.")] tab_ids: Vec<i32>,
    ) -> FieldResult<RefreshResult> {
        let mut params = serde_json::Map::new();
        params.insert(
            "tabIds".to_string(),
            serde_json::json!(tab_ids.iter().map(|&id| id as i64).collect::<Vec<_>>()),
        );

        let response = ctx
            .sender
            .send("refresh", serde_json::Value::Object(params))
            .map_err(|e| juniper::FieldError::new(e, juniper::Value::Null))?;

        let refreshed_tabs = response
            .get("summary")
            .and_then(|s| s.get("refreshedTabs"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32;

        Ok(RefreshResult { refreshed_tabs })
    }

    /// Focus (activate) a tab and bring its window to the foreground.
    fn focus_tab(
        ctx: &GqlContext,
        #[graphql(description = "ID of the tab to focus.")] tab_id: i32,
    ) -> FieldResult<FocusResult> {
        let mut params = serde_json::Map::new();
        params.insert("tabIds".to_string(), serde_json::json!([tab_id as i64]));

        ctx.sender
            .send("focus", serde_json::Value::Object(params))
            .map_err(|e| juniper::FieldError::new(e, juniper::Value::Null))?;

        Ok(FocusResult {
            success: true,
            tab_id,
        })
    }

    /// Undo a previous mutation by transaction ID.
    fn undo_action(
        ctx: &GqlContext,
        #[graphql(description = "Transaction ID to undo. Omit to undo the most recent action.")]
        txid: Option<String>,
        #[graphql(description = "When true, undo the most recent action regardless of txid.")]
        latest: Option<bool>,
    ) -> FieldResult<UndoResult> {
        let mut params = serde_json::Map::new();
        if let Some(ref id) = txid {
            params.insert("txid".to_string(), serde_json::json!(id));
        }
        if latest == Some(true) {
            params.insert("latest".to_string(), serde_json::json!(true));
        }

        let response = ctx
            .sender
            .send("undo", serde_json::Value::Object(params))
            .map_err(|e| juniper::FieldError::new(e, juniper::Value::Null))?;

        let resolved_txid = response
            .get("txid")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let summary = response
            .get("summary")
            .map(|v| v.to_string())
            .unwrap_or_default();

        Ok(UndoResult {
            txid: resolved_txid,
            summary,
        })
    }

    /// Update a tab group's properties.
    fn update_group(
        ctx: &GqlContext,
        #[graphql(description = "ID of the group to update.")] group_id: i32,
        #[graphql(description = "New title for the group.")] title: Option<String>,
        #[graphql(
            description = "New color (blue, red, yellow, green, purple, cyan, orange, grey)."
        )]
        color: Option<String>,
        #[graphql(description = "Set to true to collapse, false to expand.")] collapsed: Option<
            bool,
        >,
    ) -> FieldResult<Group> {
        let mut params = serde_json::Map::new();
        params.insert("groupId".to_string(), serde_json::json!(group_id as i64));
        if let Some(ref t) = title {
            params.insert("title".to_string(), serde_json::json!(t));
        }
        if let Some(ref c) = color {
            params.insert("color".to_string(), serde_json::json!(c));
        }
        if let Some(c) = collapsed {
            params.insert("collapsed".to_string(), serde_json::json!(c));
        }

        let response = ctx
            .sender
            .send("group-update", serde_json::Value::Object(params))
            .map_err(|e| juniper::FieldError::new(e, juniper::Value::Null))?;

        let tab_count = response
            .get("tabCount")
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32;

        Ok(Group {
            group_id,
            title: response
                .get("title")
                .and_then(|v| v.as_str())
                .or(title.as_deref())
                .unwrap_or("")
                .to_string(),
            color: response
                .get("color")
                .and_then(|v| v.as_str())
                .or(color.as_deref())
                .unwrap_or("")
                .to_string(),
            collapsed: response
                .get("collapsed")
                .and_then(|v| v.as_bool())
                .or(collapsed)
                .unwrap_or(false),
            tab_count,
        })
    }

    /// Remove tabs from their group (ungroup them).
    fn ungroup_tabs(
        ctx: &GqlContext,
        #[graphql(description = "IDs of the tabs to ungroup.")] tab_ids: Vec<i32>,
    ) -> FieldResult<Vec<Tab>> {
        let mut params = serde_json::Map::new();
        params.insert(
            "tabIds".to_string(),
            serde_json::json!(tab_ids.iter().map(|&id| id as i64).collect::<Vec<_>>()),
        );

        ctx.sender
            .send("group-ungroup", serde_json::Value::Object(params))
            .map_err(|e| juniper::FieldError::new(e, juniper::Value::Null))?;

        // Re-snapshot to get updated tab state
        let snap = ctx
            .sender
            .snapshot()
            .map_err(|e| juniper::FieldError::new(e, juniper::Value::Null))?;
        let tabs: Vec<Tab> = windows_from_snapshot(&snap)
            .into_iter()
            .flat_map(|w| w.tabs)
            .filter(|t| tab_ids.contains(&t.tab_id))
            .collect();

        Ok(tabs)
    }

    /// Assign tabs to a group, creating it if needed.
    fn assign_to_group(
        ctx: &GqlContext,
        #[graphql(description = "IDs of the tabs to assign.")] tab_ids: Vec<i32>,
        #[graphql(description = "Group title. Creates the group if it doesn't exist.")]
        group_title: String,
        #[graphql(
            description = "Group color (blue, red, yellow, green, purple, cyan, orange, grey)."
        )]
        color: Option<String>,
        #[graphql(description = "Whether the group should be collapsed.")] collapsed: Option<bool>,
    ) -> FieldResult<Group> {
        let mut params = serde_json::Map::new();
        params.insert(
            "tabIds".to_string(),
            serde_json::json!(tab_ids.iter().map(|&id| id as i64).collect::<Vec<_>>()),
        );
        params.insert("groupTitle".to_string(), serde_json::json!(group_title));
        if let Some(ref c) = color {
            params.insert("color".to_string(), serde_json::json!(c));
        }
        if let Some(c) = collapsed {
            params.insert("collapsed".to_string(), serde_json::json!(c));
        }

        let response = ctx
            .sender
            .send("group-assign", serde_json::Value::Object(params))
            .map_err(|e| juniper::FieldError::new(e, juniper::Value::Null))?;

        let group_id = response
            .get("groupId")
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32;

        Ok(Group {
            group_id,
            title: group_title,
            color: color.unwrap_or_default(),
            collapsed: collapsed.unwrap_or(false),
            tab_count: tab_ids.len() as i32,
        })
    }

    /// Move tabs to a new position, window, or group.
    fn move_tab(
        ctx: &GqlContext,
        #[graphql(description = "IDs of the tabs to move.")] tab_ids: Vec<i32>,
        #[graphql(description = "Target window ID.")] window_id: Option<i32>,
        #[graphql(description = "Target position index within the window.")] index: Option<i32>,
    ) -> FieldResult<MoveResult> {
        let mut params = serde_json::Map::new();
        params.insert(
            "tabIds".to_string(),
            serde_json::json!(tab_ids.iter().map(|&id| id as i64).collect::<Vec<_>>()),
        );
        if let Some(wid) = window_id {
            params.insert("windowId".to_string(), serde_json::json!(wid as i64));
        }
        if let Some(idx) = index {
            params.insert("index".to_string(), serde_json::json!(idx));
        }

        ctx.sender
            .send("move-tab", serde_json::Value::Object(params))
            .map_err(|e| juniper::FieldError::new(e, juniper::Value::Null))?;

        Ok(MoveResult {
            moved_tabs: tab_ids.len() as i32,
        })
    }

    /// Move a tab group to a new position or window.
    #[allow(clippy::too_many_arguments)]
    fn move_group(
        ctx: &GqlContext,
        #[graphql(description = "ID of the group to move.")] group_id: i32,
        #[graphql(description = "Target window ID. Omit to keep the current window.")]
        window_id: Option<i32>,
        #[graphql(description = "Create a new window for the moved group.")] new_window: Option<
            bool,
        >,
        #[graphql(description = "Move the group before the tabs in this named group.")]
        before_group_title: Option<String>,
        #[graphql(description = "Move the group after the tabs in this named group.")]
        after_group_title: Option<String>,
        #[graphql(description = "Move the group before this tab ID.")] before_tab_id: Option<i32>,
        #[graphql(description = "Move the group after this tab ID.")] after_tab_id: Option<i32>,
    ) -> FieldResult<MoveGroupResult> {
        let mut params = serde_json::Map::new();
        params.insert("groupId".to_string(), serde_json::json!(group_id as i64));
        if let Some(wid) = window_id {
            params.insert("targetWindowId".to_string(), serde_json::json!(wid as i64));
        }
        if new_window == Some(true) {
            params.insert("newWindow".to_string(), serde_json::json!(true));
        }
        if let Some(ref title) = before_group_title {
            params.insert("beforeGroupTitle".to_string(), serde_json::json!(title));
        }
        if let Some(ref title) = after_group_title {
            params.insert("afterGroupTitle".to_string(), serde_json::json!(title));
        }
        if let Some(tab_id) = before_tab_id {
            params.insert("beforeTabId".to_string(), serde_json::json!(tab_id as i64));
        }
        if let Some(tab_id) = after_tab_id {
            params.insert("afterTabId".to_string(), serde_json::json!(tab_id as i64));
        }

        let response = ctx
            .sender
            .send("move-group", serde_json::Value::Object(params))
            .map_err(|e| juniper::FieldError::new(e, juniper::Value::Null))?;

        Ok(MoveGroupResult {
            group_id: response
                .get("groupId")
                .and_then(|v| v.as_i64())
                .unwrap_or(group_id as i64) as i32,
            window_id: response
                .get("windowId")
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as i32,
            moved_to_window_id: response
                .get("movedToWindowId")
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as i32,
            new_group_id: response
                .get("newGroupId")
                .and_then(|v| v.as_i64())
                .map(|v| v as i32),
            moved_tabs: response
                .get("summary")
                .and_then(|s| s.get("movedTabs"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as i32,
        })
    }

    /// Merge tabs from one window into another window.
    fn merge_windows(
        ctx: &GqlContext,
        #[graphql(description = "Source window ID.")] from_window_id: i32,
        #[graphql(description = "Destination window ID.")] to_window_id: i32,
        #[graphql(description = "Close the source window when it becomes empty.")]
        close_source: Option<bool>,
        #[graphql(description = "Confirmation flag for destructive variants.")] confirm: Option<
            bool,
        >,
    ) -> FieldResult<MergeWindowsResult> {
        let mut params = serde_json::Map::new();
        params.insert(
            "fromWindowId".to_string(),
            serde_json::json!(from_window_id as i64),
        );
        params.insert(
            "toWindowId".to_string(),
            serde_json::json!(to_window_id as i64),
        );
        if close_source == Some(true) {
            params.insert("closeSource".to_string(), serde_json::json!(true));
        }
        if confirm == Some(true) {
            params.insert("confirmed".to_string(), serde_json::json!(true));
        }

        let response = ctx
            .sender
            .send("merge-window", serde_json::Value::Object(params))
            .map_err(|e| juniper::FieldError::new(e, juniper::Value::Null))?;

        Ok(MergeWindowsResult {
            from_window_id: response
                .get("fromWindowId")
                .and_then(|v| v.as_i64())
                .unwrap_or(from_window_id as i64) as i32,
            to_window_id: response
                .get("toWindowId")
                .and_then(|v| v.as_i64())
                .unwrap_or(to_window_id as i64) as i32,
            source_closed: response
                .get("sourceClosed")
                .and_then(|v| v.as_bool())
                .unwrap_or(false),
            moved_tabs: response
                .get("summary")
                .and_then(|s| s.get("movedTabs"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as i32,
            moved_groups: response
                .get("summary")
                .and_then(|s| s.get("movedGroups"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as i32,
        })
    }

    /// Merge duplicate groups with the same title.
    fn gather_groups(
        ctx: &GqlContext,
        #[graphql(description = "Restrict gathering to this window.")] window_id: Option<i32>,
        #[graphql(description = "Restrict gathering to duplicate groups with this title.")]
        group_title: Option<String>,
    ) -> FieldResult<GatherResult> {
        let mut params = serde_json::Map::new();
        if let Some(wid) = window_id {
            params.insert("windowId".to_string(), serde_json::json!(wid as i64));
        }
        if let Some(ref title) = group_title {
            params.insert("groupTitle".to_string(), serde_json::json!(title));
        }

        let response = ctx
            .sender
            .send("group-gather", serde_json::Value::Object(params))
            .map_err(|e| juniper::FieldError::new(e, juniper::Value::Null))?;

        let merged = response
            .get("merged")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .map(|entry| GatheredGroup {
                        window_id: entry.get("windowId").and_then(|v| v.as_i64()).unwrap_or(0)
                            as i32,
                        group_title: entry
                            .get("groupTitle")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string(),
                        primary_group_id: entry
                            .get("primaryGroupId")
                            .and_then(|v| v.as_i64())
                            .unwrap_or(0) as i32,
                        merged_group_count: entry
                            .get("mergedGroupCount")
                            .and_then(|v| v.as_i64())
                            .unwrap_or(0) as i32,
                        moved_tabs: entry.get("movedTabs").and_then(|v| v.as_i64()).unwrap_or(0)
                            as i32,
                    })
                    .collect()
            })
            .unwrap_or_default();

        let summary = GatherSummary {
            merged_groups: response
                .get("summary")
                .and_then(|s| s.get("mergedGroups"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as i32,
            moved_tabs: response
                .get("summary")
                .and_then(|s| s.get("movedTabs"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as i32,
        };

        Ok(GatherResult { merged, summary })
    }

    /// Archive tabs to a consolidated archive window.
    fn archive_tabs(
        ctx: &GqlContext,
        #[graphql(description = "Restrict to tabs in this window.")] window_id: Option<i32>,
        #[graphql(description = "Specific tab IDs to archive.")] tab_ids: Option<Vec<i32>>,
    ) -> FieldResult<ArchiveResult> {
        let mut params = serde_json::Map::new();
        if let Some(wid) = window_id {
            params.insert("windowId".to_string(), serde_json::json!(wid as i64));
        }
        if let Some(ref ids) = tab_ids {
            params.insert(
                "tabIds".to_string(),
                serde_json::json!(ids.iter().map(|&id| id as i64).collect::<Vec<_>>()),
            );
        }

        let response = ctx
            .sender
            .send("archive", serde_json::Value::Object(params))
            .map_err(|e| juniper::FieldError::new(e, juniper::Value::Null))?;

        let txid = response
            .get("txid")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let archived_tabs = response
            .get("summary")
            .and_then(|s| s.get("archivedTabs"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32;

        Ok(ArchiveResult {
            txid,
            archived_tabs,
        })
    }

    /// Preview or execute duplicate-tab cleanup.
    #[allow(clippy::too_many_arguments)]
    fn deduplicate_tabs(
        ctx: &GqlContext,
        #[graphql(
            description = "Number of days since last focus to consider a tab stale (default 30)."
        )]
        stale_days: Option<i32>,
        #[graphql(description = "Restrict dedupe to this window.")] window_id: Option<i32>,
        #[graphql(description = "Restrict dedupe to this group (by group ID).")] group_id: Option<
            i32,
        >,
        #[graphql(description = "Restrict dedupe to tabs whose group has this title.")]
        group_title: Option<String>,
        #[graphql(description = "When true, dedupe only ungrouped tabs (groupId = -1).")]
        ungrouped: Option<bool>,
        #[graphql(description = "Actually close duplicate tabs instead of previewing only.")]
        confirm: Option<bool>,
    ) -> FieldResult<DedupeResult> {
        let mut params = serde_json::Map::new();
        params.insert("dedupe".to_string(), serde_json::json!(true));
        if let Some(days) = stale_days {
            params.insert("staleDays".to_string(), serde_json::json!(days));
        }
        if let Some(wid) = window_id {
            params.insert("windowId".to_string(), serde_json::json!(wid as i64));
        }
        if let Some(gid) = group_id {
            params.insert("groupId".to_string(), serde_json::json!(gid as i64));
        }
        if let Some(ref title) = group_title {
            params.insert("groupTitle".to_string(), serde_json::json!(title));
        }
        if ungrouped == Some(true) {
            params.insert("ungrouped".to_string(), serde_json::json!(true));
        }
        if confirm == Some(true) {
            params.insert("confirmed".to_string(), serde_json::json!(true));
        }

        let response = ctx
            .sender
            .send("analyze", serde_json::Value::Object(params))
            .map_err(|e| juniper::FieldError::new(e, juniper::Value::Null))?;

        Ok(DedupeResult {
            txid: response
                .get("txid")
                .and_then(|v| v.as_str())
                .map(|v| v.to_string()),
            closed_tabs: response
                .get("dedupeSummary")
                .and_then(|s| s.get("closedTabs"))
                .and_then(|v| v.as_i64())
                .unwrap_or(0) as i32,
            duplicate_groups: response
                .get("duplicates")
                .and_then(|v| v.as_array())
                .map(|arr| arr.len())
                .unwrap_or(0) as i32,
            candidate_tabs: duplicate_candidate_tabs(&response),
        })
    }

    /// Reload the connected browser extension.
    fn reload_extension(ctx: &GqlContext) -> FieldResult<ReloadResult> {
        let response = ctx
            .sender
            .send("reload", serde_json::json!({}))
            .map_err(|e| juniper::FieldError::new(e, juniper::Value::Null))?;

        Ok(ReloadResult {
            reloading: response
                .get("reloading")
                .and_then(|v| v.as_bool())
                .unwrap_or(true),
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::context::CommandSender;
    use std::sync::Arc;

    struct MockSender;

    impl CommandSender for MockSender {
        fn send(
            &self,
            action: &str,
            params: serde_json::Value,
        ) -> Result<serde_json::Value, String> {
            match action {
                "close" => Ok(serde_json::json!({
                    "txid": "tx-test-1",
                    "summary": { "closedTabs": 2, "skippedTabs": 0 },
                    "skipped": []
                })),
                "open" => Ok(serde_json::json!({
                    "windowId": 100,
                    "groupId": 10,
                    "created": [
                        {"tabId": 50, "windowId": 100, "index": 3, "url": "https://new.com", "title": "New"},
                        {"tabId": 51, "windowId": 100, "index": 4, "url": "https://new2.com", "title": "New2"}
                    ],
                    "createdTabIds": [50, 51],
                    "skipped": [
                        {"url": "https://dup.com", "reason": "duplicate"}
                    ],
                    "summary": {
                        "createdTabs": 2,
                        "skippedUrls": 1,
                        "grouped": true
                    }
                })),
                "ping" => Ok(serde_json::json!({ "ok": true })),
                "analyze" => {
                    let dedupe = params.get("dedupe").and_then(|v| v.as_bool()) == Some(true);
                    let confirmed = params.get("confirmed").and_then(|v| v.as_bool()) == Some(true);
                    Ok(serde_json::json!({
                        "txid": if dedupe && confirmed { serde_json::Value::String("tx-dedupe-1".into()) } else { serde_json::Value::Null },
                        "stale": [{"tabId": 4}],
                        "duplicates": [{
                            "normalizedUrl": "example.com/page",
                            "tabs": [
                                {"tabId": 2, "windowId": 100, "index": 1, "url": "https://example.com/page", "title": "B", "active": false, "pinned": true, "groupId": -1},
                                {"tabId": 4, "windowId": 200, "index": 0, "url": "https://example.com/page", "title": "D", "active": true, "pinned": false, "groupId": -1}
                            ]
                        }],
                        "summary": { "totalTabs": 4 },
                        "dedupeSummary": if dedupe && confirmed { serde_json::json!({"closedTabs": 1}) } else { serde_json::Value::Null }
                    }))
                }
                "history" => Ok(serde_json::json!([
                    {"txid": "tx-1", "action": "close", "summary": {"closedTabs": 1}, "createdAt": 1700000000000.0}
                ])),
                "browser-state-history" => Ok(serde_json::json!([
                    {
                        "snapshotId": 7,
                        "recordedAt": 1700000001111.0,
                        "reason": "event",
                        "eventCount": 2,
                        "eventKinds": ["tabs.onMoved", "tabGroups.onUpdated"],
                        "previousSnapshotId": 6,
                        "windowCount": 1,
                        "groupCount": 1,
                        "tabCount": 2
                    }
                ])),
                "browser-state-latest" => Ok(serde_json::json!({
                    "snapshotId": 7,
                    "recordedAt": 1700000001111.0,
                    "reason": "event",
                    "eventCount": 2,
                    "eventKinds": ["tabs.onMoved", "tabGroups.onUpdated"],
                    "previousSnapshotId": 6,
                    "snapshot": sample_snapshot(),
                    "groups": [{
                        "logicalGroupId": "grp-123",
                        "logicalWindowId": "win-456",
                        "browserGroupId": 10,
                        "browserWindowId": 100,
                        "windowOrdinal": 0,
                        "title": "Work",
                        "color": "blue",
                        "collapsed": false,
                        "tabCount": 2,
                        "tabUrls": ["a.com", "b.com"]
                    }]
                })),
                "browser-state-events" => Ok(serde_json::json!([
                    {
                        "eventId": 11,
                        "recordedAt": 1700000001111.0,
                        "reason": "event",
                        "beforeSnapshotId": 6,
                        "afterSnapshotId": 7,
                        "kind": "tabGroups.onUpdated",
                        "browserWindowId": 100,
                        "browserGroupId": 10,
                        "browserTabId": serde_json::Value::Null,
                        "payloadJson": "{\"kind\":\"tabGroups.onUpdated\"}"
                    }
                ])),
                "browser-state-group-history" => Ok(serde_json::json!([
                    {
                        "logicalGroupId": "grp-123",
                        "logicalWindowId": "win-456",
                        "browserGroupId": 10,
                        "browserWindowId": 100,
                        "windowOrdinal": 0,
                        "title": "Work",
                        "color": "blue",
                        "collapsed": false,
                        "tabCount": 2,
                        "tabUrls": ["a.com", "b.com"],
                        "snapshotId": 7,
                        "recordedAt": 1700000001111.0,
                        "reason": "event"
                    }
                ])),
                "focus" => Ok(serde_json::json!({ "tabId": 1, "windowId": 100 })),
                "undo" => Ok(serde_json::json!({
                    "txid": "tx-test-1",
                    "summary": "Reopened 2 tabs"
                })),
                "refresh" => Ok(serde_json::json!({
                    "summary": { "refreshedTabs": 2 }
                })),
                "group-update" => Ok(serde_json::json!({
                    "groupId": 10,
                    "windowId": 100,
                    "txid": "tx-gu-1",
                    "summary": { "updatedGroups": 1 }
                })),
                "group-ungroup" => Ok(serde_json::json!({
                    "groupId": 10,
                    "windowId": 100,
                    "txid": "tx-uu-1",
                    "summary": { "ungroupedTabs": 1 }
                })),
                "group-assign" => Ok(serde_json::json!({
                    "groupId": 20,
                    "windowId": 100,
                    "created": false,
                    "txid": "tx-ga-1",
                    "summary": { "movedTabs": 1, "groupedTabs": 1, "skippedTabs": 0 },
                    "skipped": []
                })),
                "move-tab" => Ok(serde_json::json!({
                    "tabId": 1,
                    "fromWindowId": 100,
                    "toWindowId": 200,
                    "toIndex": 0,
                    "txid": "tx-mv-1",
                    "summary": { "movedTabs": 1 }
                })),
                "move-group" => Ok(serde_json::json!({
                    "groupId": 20,
                    "windowId": 100,
                    "movedToWindowId": 200,
                    "newGroupId": 50,
                    "summary": { "movedTabs": 2 }
                })),
                "merge-window" => Ok(serde_json::json!({
                    "fromWindowId": 100,
                    "toWindowId": 200,
                    "sourceClosed": true,
                    "summary": { "movedTabs": 3, "movedGroups": 1 }
                })),
                "group-gather" => Ok(serde_json::json!({
                    "merged": [{
                        "windowId": 100,
                        "groupTitle": "Work",
                        "primaryGroupId": 10,
                        "mergedGroupCount": 1,
                        "movedTabs": 2
                    }],
                    "summary": { "mergedGroups": 1, "movedTabs": 2 }
                })),
                "archive" => Ok(serde_json::json!({
                    "txid": "tx-archive-1",
                    "archiveWindowId": 300,
                    "summary": { "archivedTabs": 3, "archivedGroups": 1, "movedTabs": 3 }
                })),
                "inspect" => Ok(serde_json::json!({
                    "totals": { "tabs": 1, "signals": 2, "tasks": 2 },
                    "entries": [{
                        "tabId": 1,
                        "windowId": 100,
                        "url": "https://a.com",
                        "title": "A",
                        "signals": {
                            "page-meta": { "description": "Page desc" },
                            "price": { "price": "$9.99" }
                        }
                    }]
                })),
                "report" => Ok(serde_json::json!({
                    "generatedAt": 1700000001234.0,
                    "entries": [{
                        "tabId": 1,
                        "windowId": 100,
                        "url": "https://a.com",
                        "title": "A",
                        "groupId": 10,
                        "groupTitle": "Work",
                        "groupColor": "blue",
                        "description": "Page desc",
                        "lastAccessedAt": 1700000000000.0
                    }],
                    "totals": { "tabs": 1 }
                })),
                "screenshot" => Ok(serde_json::json!({
                    "totals": { "tabs": 1, "tiles": 1 },
                    "entries": [{
                        "tabId": 1,
                        "windowId": 100,
                        "groupId": -1,
                        "url": "https://a.com",
                        "title": "A",
                        "tiles": [{
                            "index": 0,
                            "total": 1,
                            "x": 0,
                            "y": 0,
                            "width": 100,
                            "height": 120,
                            "scale": 2.0,
                            "bytes": 1234,
                            "scaled": false,
                            "oversized": false,
                            "dataUrl": "data:image/png;base64,abc"
                        }]
                    }]
                })),
                "reload" => Ok(serde_json::json!({ "reloading": true })),
                _ => Ok(serde_json::json!({
                    "txid": "tx-test-1",
                    "summary": {}
                })),
            }
        }

        fn snapshot(&self) -> Result<serde_json::Value, String> {
            Ok(sample_snapshot())
        }
    }

    fn sample_snapshot() -> serde_json::Value {
        serde_json::json!({
            "windows": [{
                "windowId": 100,
                "focused": true,
                "tabs": [
                    {"tabId": 1, "windowId": 100, "index": 0, "url": "https://a.com", "title": "A", "active": true, "pinned": false, "groupId": 10, "groupTitle": "Work", "groupColor": "blue", "groupCollapsed": false, "lastAccessedAt": 1700000000000.0},
                    {"tabId": 2, "windowId": 100, "index": 1, "url": "https://b.com", "title": "B", "active": false, "pinned": true, "groupId": -1},
                    {"tabId": 3, "windowId": 100, "index": 2, "url": "https://c.com", "title": "C", "active": false, "pinned": false, "groupId": 10, "groupTitle": "Work", "groupColor": "blue", "groupCollapsed": false}
                ],
                "groups": [{"groupId": 10, "title": "Work", "color": "blue", "collapsed": false}]
            }, {
                "windowId": 200,
                "focused": false,
                "tabs": [
                    {"tabId": 4, "windowId": 200, "index": 0, "url": "https://d.com", "title": "D", "active": true, "pinned": false, "groupId": -1}
                ],
                "groups": []
            }]
        })
    }

    fn exec(query: &str) -> serde_json::Value {
        crate::execute(query, None, sample_snapshot(), Arc::new(MockSender)).unwrap()
    }

    #[test]
    fn query_windows_returns_all() {
        let result = exec("{ windows { windowId focused tabCount } }");
        let windows = result["data"]["windows"].as_array().unwrap();
        assert_eq!(windows.len(), 2);
    }

    #[test]
    fn query_window_by_id() {
        let result = exec("{ window(id: 100) { windowId tabs { tabId url } } }");
        assert!(!result["data"]["window"].is_null());
        let tabs = result["data"]["window"]["tabs"].as_array().unwrap();
        assert_eq!(tabs.len(), 3);
    }

    #[test]
    fn query_tabs_with_group_title_filter() {
        let result = exec(r#"{ tabs(groupTitle: "Work") { items { tabId } total hasMore } }"#);
        let tabs = result["data"]["tabs"]["items"].as_array().unwrap();
        assert_eq!(tabs.len(), 2);
        assert_eq!(result["data"]["tabs"]["total"], 2);
        assert_eq!(result["data"]["tabs"]["hasMore"], false);
    }

    #[test]
    fn query_tabs_ungrouped() {
        let result = exec("{ tabs(ungrouped: true) { items { tabId } total } }");
        let tabs = result["data"]["tabs"]["items"].as_array().unwrap();
        assert_eq!(tabs.len(), 2);
    }

    #[test]
    fn query_tab_by_id() {
        let result = exec("{ tab(id: 2) { tabId url title } }");
        assert!(!result["data"]["tab"].is_null());
        assert_eq!(result["data"]["tab"]["url"], "https://b.com");
    }

    #[test]
    fn query_groups_filtered_by_window() {
        let result = exec("{ groups(windowId: 100) { groupId title tabCount } }");
        let groups = result["data"]["groups"].as_array().unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0]["tabCount"], 2);
    }

    #[test]
    fn introspection_returns_tab_type() {
        let result = exec(r#"{ __type(name: "Tab") { name fields { name description } } }"#);
        assert!(!result["data"]["__type"].is_null());
        let fields = result["data"]["__type"]["fields"].as_array().unwrap();
        let field_names: Vec<&str> = fields.iter().filter_map(|f| f["name"].as_str()).collect();
        assert!(field_names.contains(&"tabId"));
        assert!(field_names.contains(&"url"));
        assert!(field_names.contains(&"title"));
        assert!(field_names.contains(&"lastAccessedAt"));
        assert!(field_names.contains(&"groupColor"));
        assert!(field_names.contains(&"groupCollapsed"));

        // Verify descriptions are populated
        for field in fields {
            assert!(
                field["description"].as_str().is_some_and(|d| !d.is_empty()),
                "Field {} should have a description",
                field["name"]
            );
        }
    }

    #[test]
    fn tabs_default_limit_and_pagination() {
        let result = exec("{ tabs { items { tabId } total offset hasMore } }");
        let page = &result["data"]["tabs"];
        let items = page["items"].as_array().unwrap();
        // Sample snapshot has 4 tabs total, all fit in default limit 20
        assert_eq!(items.len(), 4);
        assert_eq!(page["total"], 4);
        assert_eq!(page["offset"], 0);
        assert_eq!(page["hasMore"], false);
    }

    #[test]
    fn tabs_limit_and_offset() {
        let result = exec("{ tabs(limit: 2, offset: 1) { items { tabId } total offset hasMore } }");
        let page = &result["data"]["tabs"];
        let items = page["items"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(page["total"], 4);
        assert_eq!(page["offset"], 1);
        assert_eq!(page["hasMore"], true);
    }

    #[test]
    fn mutation_close_tabs() {
        let result =
            exec("mutation { closeTabs(tabIds: [1, 2], confirm: true) { txid closedTabs } }");
        assert!(result.get("errors").is_none());
        assert_eq!(result["data"]["closeTabs"]["txid"], "tx-test-1");
        assert_eq!(result["data"]["closeTabs"]["closedTabs"], 2);
    }

    #[test]
    fn schema_sdl_contains_types() {
        let sdl = crate::schema_sdl();
        assert!(sdl.contains("type Tab"));
        assert!(sdl.contains("type Window"));
        assert!(sdl.contains("type Group"));
        assert!(sdl.contains("type Query"));
        assert!(sdl.contains("type Mutation"));
    }

    #[test]
    fn query_tab_returns_new_fields() {
        let result = exec("{ tab(id: 1) { tabId groupColor groupCollapsed lastAccessedAt } }");
        let tab = &result["data"]["tab"];
        assert_eq!(tab["groupColor"], "blue");
        assert_eq!(tab["groupCollapsed"], false);
        assert_eq!(tab["lastAccessedAt"], 1700000000000.0);

        // Ungrouped tab has null group metadata
        let result2 = exec("{ tab(id: 2) { tabId groupColor groupCollapsed lastAccessedAt } }");
        let tab2 = &result2["data"]["tab"];
        assert!(tab2["groupColor"].is_null());
        assert!(tab2["groupCollapsed"].is_null());
        assert!(tab2["lastAccessedAt"].is_null());
    }

    #[test]
    fn query_ping() {
        let result = exec("{ ping { ok latencyMs } }");
        assert!(result.get("errors").is_none());
        assert_eq!(result["data"]["ping"]["ok"], true);
        assert!(result["data"]["ping"]["latencyMs"].as_f64().unwrap() >= 0.0);
    }

    #[test]
    fn query_analyze() {
        let result = exec("{ analyze { staleTabs duplicateTabs totalTabs } }");
        assert!(result.get("errors").is_none());
        assert_eq!(result["data"]["analyze"]["staleTabs"], 1);
        assert_eq!(result["data"]["analyze"]["duplicateTabs"], 1);
        assert_eq!(result["data"]["analyze"]["totalTabs"], 4);
    }

    #[test]
    fn query_history() {
        let result = exec("{ history { txid action createdAt } }");
        assert!(result.get("errors").is_none());
        let entries = result["data"]["history"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["txid"], "tx-1");
        assert_eq!(entries[0]["action"], "close");
    }

    #[test]
    fn query_browser_state_history() {
        let result = exec(
            "{ browserStateHistory { snapshotId reason eventCount eventKinds previousSnapshotId groupCount } }",
        );
        assert!(result.get("errors").is_none());
        let entries = result["data"]["browserStateHistory"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["snapshotId"], 7);
        assert_eq!(entries[0]["reason"], "event");
        assert_eq!(entries[0]["eventCount"], 2);
        assert_eq!(entries[0]["previousSnapshotId"], 6);
    }

    #[test]
    fn query_latest_browser_state() {
        let result = exec(
            "{ latestBrowserState { snapshotId reason eventKinds windows { windowId tabs { tabId } } groups { logicalGroupId title tabUrls } } }",
        );
        assert!(result.get("errors").is_none());
        let latest = &result["data"]["latestBrowserState"];
        assert_eq!(latest["snapshotId"], 7);
        assert_eq!(latest["reason"], "event");
        assert_eq!(latest["windows"][0]["windowId"], 100);
        assert_eq!(latest["groups"][0]["logicalGroupId"], "grp-123");
    }

    #[test]
    fn query_browser_state_events() {
        let result = exec(
            "{ browserStateEvents { eventId kind beforeSnapshotId afterSnapshotId payloadJson } }",
        );
        assert!(result.get("errors").is_none());
        let entries = result["data"]["browserStateEvents"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["eventId"], 11);
        assert_eq!(entries[0]["kind"], "tabGroups.onUpdated");
        assert_eq!(entries[0]["beforeSnapshotId"], 6);
    }

    #[test]
    fn query_browser_state_group_history() {
        let result = exec(
            "{ browserStateGroupHistory(title: \"Work\") { logicalGroupId title browserGroupId browserWindowId tabUrls } }",
        );
        assert!(result.get("errors").is_none());
        let entries = result["data"]["browserStateGroupHistory"]
            .as_array()
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["logicalGroupId"], "grp-123");
        assert_eq!(entries[0]["title"], "Work");
    }

    #[test]
    fn query_inspect_tabs() {
        let result = exec(
            r#"{ inspectTabs(windowId: 100, signals: ["page-meta"], selectors: [{name: "price", selector: ".price", attr: "text"}]) {
                totals { tabs signals tasks }
                entries { tabId windowId signals { name valueJson } }
            } }"#,
        );
        assert!(result.get("errors").is_none());
        let inspect = &result["data"]["inspectTabs"];
        assert_eq!(inspect["totals"]["tabs"], 1);
        assert_eq!(inspect["totals"]["signals"], 2);
        assert_eq!(inspect["totals"]["tasks"], 2);
        let entries = inspect["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        let signals = entries[0]["signals"].as_array().unwrap();
        assert!(signals.iter().any(|s| s["name"] == "page-meta"));
        assert!(signals.iter().any(|s| s["name"] == "price"));
    }

    #[test]
    fn query_report_tabs() {
        let result = exec(
            "{ reportTabs(windowId: 100) { generatedAt totals { tabs } entries { tabId description groupColor } } }",
        );
        assert!(result.get("errors").is_none());
        let report = &result["data"]["reportTabs"];
        assert_eq!(report["totals"]["tabs"], 1);
        let entries = report["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["tabId"], 1);
        assert_eq!(entries[0]["description"], "Page desc");
        assert_eq!(entries[0]["groupColor"], "blue");
    }

    #[test]
    fn query_capture_screenshots() {
        let result = exec(
            "{ captureScreenshots(windowId: 100, mode: \"viewport\") { totals { tabs tiles } entries { tabId tiles { width height dataUrl } } } }",
        );
        assert!(result.get("errors").is_none());
        let capture = &result["data"]["captureScreenshots"];
        assert_eq!(capture["totals"]["tabs"], 1);
        assert_eq!(capture["totals"]["tiles"], 1);
        let entries = capture["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        let tiles = entries[0]["tiles"].as_array().unwrap();
        assert_eq!(tiles.len(), 1);
        assert_eq!(tiles[0]["width"], 100);
        assert_eq!(tiles[0]["height"], 120);
    }

    #[test]
    fn mutation_focus_tab() {
        let result = exec("mutation { focusTab(tabId: 1) { success tabId } }");
        assert!(result.get("errors").is_none());
        assert_eq!(result["data"]["focusTab"]["success"], true);
        assert_eq!(result["data"]["focusTab"]["tabId"], 1);
    }

    #[test]
    fn mutation_undo_action() {
        let result = exec(r#"mutation { undoAction(txid: "tx-test-1") { txid summary } }"#);
        assert!(result.get("errors").is_none());
        assert_eq!(result["data"]["undoAction"]["txid"], "tx-test-1");
    }

    #[test]
    fn mutation_update_group() {
        let result = exec(
            r#"mutation { updateGroup(groupId: 10, title: "Updated", color: "red", collapsed: true) { groupId title color collapsed tabCount } }"#,
        );
        assert!(result.get("errors").is_none());
        let group = &result["data"]["updateGroup"];
        assert_eq!(group["groupId"], 10);
        // Real orchestration doesn't return title/color/collapsed — resolver
        // falls back to the input parameters via .or() chains.
        assert_eq!(group["title"], "Updated");
        assert_eq!(group["color"], "red");
        assert_eq!(group["collapsed"], true);
        // Real orchestration doesn't return tabCount; resolver defaults to 0.
        assert_eq!(group["tabCount"], 0);
    }

    #[test]
    fn mutation_ungroup_tabs() {
        let result = exec("mutation { ungroupTabs(tabIds: [1]) { tabId groupId } }");
        assert!(result.get("errors").is_none());
        let tabs = result["data"]["ungroupTabs"].as_array().unwrap();
        assert_eq!(tabs.len(), 1);
    }

    #[test]
    fn mutation_move_tab() {
        let result = exec("mutation { moveTab(tabIds: [1, 2]) { movedTabs } }");
        assert!(result.get("errors").is_none());
        assert_eq!(result["data"]["moveTab"]["movedTabs"], 2);
    }

    #[test]
    fn mutation_move_group() {
        let result = exec(
            r#"mutation { moveGroup(groupId: 10, windowId: 200, afterGroupTitle: "Anchor") { groupId windowId movedToWindowId newGroupId movedTabs } }"#,
        );
        assert!(result.get("errors").is_none());
        let moved = &result["data"]["moveGroup"];
        assert_eq!(moved["groupId"], 20);
        assert_eq!(moved["windowId"], 100);
        assert_eq!(moved["movedToWindowId"], 200);
        assert_eq!(moved["newGroupId"], 50);
        assert_eq!(moved["movedTabs"], 2);
    }

    #[test]
    fn mutation_merge_windows() {
        let result = exec(
            "mutation { mergeWindows(fromWindowId: 100, toWindowId: 200, closeSource: true) { fromWindowId toWindowId sourceClosed movedTabs movedGroups } }",
        );
        assert!(result.get("errors").is_none());
        let merged = &result["data"]["mergeWindows"];
        assert_eq!(merged["fromWindowId"], 100);
        assert_eq!(merged["toWindowId"], 200);
        assert_eq!(merged["sourceClosed"], true);
        assert_eq!(merged["movedTabs"], 3);
        assert_eq!(merged["movedGroups"], 1);
    }

    #[test]
    fn mutation_gather_groups() {
        let result = exec(
            r#"mutation { gatherGroups(windowId: 100, groupTitle: "Work") { summary { mergedGroups movedTabs } merged { groupTitle primaryGroupId } } }"#,
        );
        assert!(result.get("errors").is_none());
        let gathered = &result["data"]["gatherGroups"];
        assert_eq!(gathered["summary"]["mergedGroups"], 1);
        assert_eq!(gathered["summary"]["movedTabs"], 2);
        let merged = gathered["merged"].as_array().unwrap();
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0]["groupTitle"], "Work");
        assert_eq!(merged[0]["primaryGroupId"], 10);
    }

    #[test]
    fn mutation_archive_tabs() {
        let result = exec("mutation { archiveTabs(windowId: 100) { txid archivedTabs } }");
        assert!(result.get("errors").is_none());
        assert_eq!(result["data"]["archiveTabs"]["txid"], "tx-archive-1");
        assert_eq!(result["data"]["archiveTabs"]["archivedTabs"], 3);
    }

    #[test]
    fn schema_sdl_contains_new_types() {
        let sdl = crate::schema_sdl();
        assert!(sdl.contains("type FocusResult"));
        assert!(sdl.contains("type UndoResult"));
        assert!(sdl.contains("type MoveResult"));
        assert!(sdl.contains("type MoveGroupResult"));
        assert!(sdl.contains("type MergeWindowsResult"));
        assert!(sdl.contains("type GatherResult"));
        assert!(sdl.contains("type GatherSummary"));
        assert!(sdl.contains("type ArchiveResult"));
        assert!(sdl.contains("type AnalyzeResult"));
        assert!(sdl.contains("type DedupeResult"));
        assert!(sdl.contains("type InspectResult"));
        assert!(sdl.contains("type ReportResult"));
        assert!(sdl.contains("type ScreenshotResult"));
        assert!(sdl.contains("type PingResult"));
        assert!(sdl.contains("type HistoryEntry"));
        assert!(sdl.contains("type ReloadResult"));
        assert!(sdl.contains("type SkippedUrl"));
        assert!(sdl.contains("type OpenResult"));
        assert!(sdl.contains("input SelectorSpecInput"));
    }

    #[test]
    fn mutation_open_tabs() {
        let result = exec(
            r#"mutation { openTabs(urls: ["https://new.com", "https://new2.com"], group: "Work") { tabs { tabId url title } skippedUrls { url reason } windowId groupId } }"#,
        );
        assert!(result.get("errors").is_none());
        let open = &result["data"]["openTabs"];
        let tabs = open["tabs"].as_array().unwrap();
        assert_eq!(tabs.len(), 2);
        assert_eq!(tabs[0]["tabId"], 50);
        assert_eq!(tabs[0]["url"], "https://new.com");
        assert_eq!(tabs[1]["tabId"], 51);
        assert_eq!(tabs[1]["url"], "https://new2.com");
        // Skipped URLs
        let skipped = open["skippedUrls"].as_array().unwrap();
        assert_eq!(skipped.len(), 1);
        assert_eq!(skipped[0]["url"], "https://dup.com");
        assert_eq!(skipped[0]["reason"], "duplicate");
        // Window and group IDs
        assert_eq!(open["windowId"], 100);
        assert_eq!(open["groupId"], 10);
    }

    #[test]
    fn mutation_deduplicate_tabs_preview() {
        let result = exec(
            "mutation { deduplicateTabs(windowId: 100) { txid closedTabs duplicateGroups candidateTabs { tabId url } } }",
        );
        assert!(result.get("errors").is_none());
        let dedupe = &result["data"]["deduplicateTabs"];
        assert!(dedupe["txid"].is_null());
        assert_eq!(dedupe["closedTabs"], 0);
        assert_eq!(dedupe["duplicateGroups"], 1);
        let candidate_tabs = dedupe["candidateTabs"].as_array().unwrap();
        assert_eq!(candidate_tabs.len(), 1);
        assert_eq!(candidate_tabs[0]["tabId"], 4);
    }

    #[test]
    fn mutation_deduplicate_tabs_confirmed() {
        let result = exec(
            "mutation { deduplicateTabs(confirm: true) { txid closedTabs duplicateGroups candidateTabs { tabId } } }",
        );
        assert!(result.get("errors").is_none());
        let dedupe = &result["data"]["deduplicateTabs"];
        assert_eq!(dedupe["txid"], "tx-dedupe-1");
        assert_eq!(dedupe["closedTabs"], 1);
        assert_eq!(dedupe["duplicateGroups"], 1);
    }

    #[test]
    fn mutation_reload_extension() {
        let result = exec("mutation { reloadExtension { reloading } }");
        assert!(result.get("errors").is_none());
        assert_eq!(result["data"]["reloadExtension"]["reloading"], true);
    }

    #[test]
    fn mutation_refresh_tabs() {
        let result = exec("mutation { refreshTabs(tabIds: [1, 2]) { refreshedTabs } }");
        assert!(result.get("errors").is_none());
        assert_eq!(result["data"]["refreshTabs"]["refreshedTabs"], 2);
    }

    #[test]
    fn mutation_assign_to_group() {
        let result = exec(
            r#"mutation { assignToGroup(tabIds: [1, 2], groupTitle: "Dev") { groupId title } }"#,
        );
        assert!(result.get("errors").is_none());
        let group = &result["data"]["assignToGroup"];
        assert_eq!(group["groupId"], 20);
        assert_eq!(group["title"], "Dev");
    }
}
