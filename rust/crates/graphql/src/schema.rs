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
    ) -> FieldResult<AnalyzeResult> {
        let mut params = serde_json::Map::new();
        if let Some(days) = stale_days {
            params.insert("staleDays".to_string(), serde_json::json!(days));
        }
        if let Some(wid) = window_id {
            params.insert("windowId".to_string(), serde_json::json!(wid as i64));
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
            .get("totalTabs")
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
    ) -> FieldResult<OpenResult> {
        let mut params = serde_json::Map::new();
        params.insert("urls".to_string(), serde_json::json!(urls));
        if let Some(wid) = window_id {
            params.insert("windowId".to_string(), serde_json::json!(wid as i64));
        }
        if let Some(ref g) = group {
            params.insert("groupTitle".to_string(), serde_json::json!(g));
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
            _params: serde_json::Value,
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
                "analyze" => Ok(serde_json::json!({
                    "stale": [{"tabId": 4}],
                    "duplicates": [],
                    "totalTabs": 4
                })),
                "history" => Ok(serde_json::json!([
                    {"txid": "tx-1", "action": "close", "summary": {"closedTabs": 1}, "createdAt": 1700000000000.0}
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
                "archive" => Ok(serde_json::json!({
                    "txid": "tx-archive-1",
                    "archiveWindowId": 300,
                    "summary": { "archivedTabs": 3, "archivedGroups": 1, "movedTabs": 3 }
                })),
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
        assert_eq!(result["data"]["analyze"]["duplicateTabs"], 0);
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
        assert!(sdl.contains("type ArchiveResult"));
        assert!(sdl.contains("type AnalyzeResult"));
        assert!(sdl.contains("type PingResult"));
        assert!(sdl.contains("type HistoryEntry"));
        assert!(sdl.contains("type SkippedUrl"));
        assert!(sdl.contains("type OpenResult"));
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
