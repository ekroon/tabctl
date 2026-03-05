use juniper::GraphQLObject;

/// A browser tab.
#[derive(Debug, Clone, GraphQLObject)]
pub(crate) struct Tab {
    /// Chrome-assigned tab identifier, unique within the browser session.
    pub tab_id: i32,
    /// Identifier of the window containing this tab.
    pub window_id: i32,
    /// Full URL of the page loaded in this tab.
    pub url: String,
    /// Page title as shown in the tab strip.
    pub title: String,
    /// Whether this tab is the active (focused) tab in its window.
    pub active: bool,
    /// Group identifier. -1 means the tab is ungrouped.
    pub group_id: i32,
    /// Title of the tab's group, if it belongs to one.
    pub group_title: Option<String>,
    /// Color of the tab's group (blue, red, yellow, green, purple, cyan, orange, grey), if grouped.
    pub group_color: Option<String>,
    /// Whether the tab's group is collapsed, if grouped.
    pub group_collapsed: Option<bool>,
    /// Whether this tab is pinned to the left of the tab strip.
    pub pinned: bool,
    /// Zero-based position of the tab within its window.
    pub index: i32,
    /// Unix timestamp (ms) when this tab was last focused, or null if never focused since the extension loaded.
    pub last_focused_at: Option<f64>,
    /// URL of the tab's favicon, if available.
    pub fav_icon_url: Option<String>,
    /// Loading status: "loading" or "complete".
    pub status: Option<String>,
    /// Whether the tab has been discarded (unloaded from memory to save resources).
    pub discarded: bool,
    /// Whether the tab is currently producing audio.
    pub audible: bool,
}

/// A browser window.
#[derive(Debug, Clone, GraphQLObject)]
pub(crate) struct Window {
    /// Chrome-assigned window identifier.
    pub window_id: i32,
    /// Whether this window currently has OS focus.
    pub focused: bool,
    /// All tabs in this window, ordered by index.
    pub tabs: Vec<Tab>,
    /// Tab groups defined in this window.
    pub groups: Vec<Group>,
    /// Total number of tabs in this window.
    pub tab_count: i32,
}

/// A tab group within a window.
#[derive(Debug, Clone, GraphQLObject)]
pub(crate) struct Group {
    /// Chrome-assigned group identifier.
    pub group_id: i32,
    /// Display name of the group.
    pub title: String,
    /// Group color: blue, red, yellow, green, purple, cyan, orange, or grey.
    pub color: String,
    /// Whether the group is collapsed (tabs hidden in the tab strip).
    pub collapsed: bool,
    /// Number of tabs currently in this group.
    pub tab_count: i32,
}

/// Result of a closeTabs mutation.
#[derive(Debug, Clone, GraphQLObject)]
pub(crate) struct CloseResult {
    /// Transaction identifier — pass to `undo` to reverse the close.
    pub txid: String,
    /// Number of tabs actually closed.
    pub closed_tabs: i32,
    /// Tabs remaining in the browser after the close.
    pub remaining_tabs: Vec<Tab>,
}

/// Result of an openTabs mutation.
#[derive(Debug, Clone, GraphQLObject)]
pub(crate) struct OpenResult {
    /// The newly created tabs.
    pub tabs: Vec<Tab>,
}

/// Result of a refreshTabs mutation.
#[derive(Debug, Clone, GraphQLObject)]
pub(crate) struct RefreshResult {
    /// Number of tabs that were reloaded.
    pub refreshed_tabs: i32,
}

/// Paginated list of tabs.
#[derive(Debug, Clone, GraphQLObject)]
pub(crate) struct TabPage {
    /// Tabs on this page.
    pub items: Vec<Tab>,
    /// Total number of tabs matching the filters (before pagination).
    pub total: i32,
    /// Zero-based offset of the first item on this page.
    pub offset: i32,
    /// Whether more tabs exist beyond this page.
    pub has_more: bool,
}

/// Result of an undoAction mutation.
#[derive(Debug, Clone, GraphQLObject)]
pub(crate) struct UndoResult {
    /// Transaction ID that was undone.
    pub txid: String,
    /// Human-readable summary of what was reversed.
    pub summary: String,
}

/// Result of a focusTab mutation.
#[derive(Debug, Clone, GraphQLObject)]
pub(crate) struct FocusResult {
    /// Whether the tab was successfully focused.
    pub success: bool,
    /// The tab ID that was focused.
    pub tab_id: i32,
}

/// Result of a moveTab mutation.
#[derive(Debug, Clone, GraphQLObject)]
pub(crate) struct MoveResult {
    /// Number of tabs moved.
    pub moved_tabs: i32,
}

/// Result of an archiveTabs mutation.
#[derive(Debug, Clone, GraphQLObject)]
pub(crate) struct ArchiveResult {
    /// Transaction ID — pass to `undo` to reverse the archive.
    pub txid: String,
    /// Number of tabs archived.
    pub archived_tabs: i32,
}

/// Result of an analyze query.
#[derive(Debug, Clone, GraphQLObject)]
pub(crate) struct AnalyzeResult {
    /// Number of stale tabs detected.
    pub stale_tabs: i32,
    /// Number of duplicate tabs detected.
    pub duplicate_tabs: i32,
    /// Total tabs analyzed.
    pub total_tabs: i32,
    /// Raw JSON analysis data.
    pub raw: String,
}

/// Result of a ping query.
#[derive(Debug, Clone, GraphQLObject)]
pub(crate) struct PingResult {
    /// Whether the host responded.
    pub ok: bool,
    /// Round-trip latency in milliseconds.
    pub latency_ms: f64,
}

/// A single history entry.
#[derive(Debug, Clone, GraphQLObject)]
pub(crate) struct HistoryEntry {
    /// Transaction ID.
    pub txid: String,
    /// Action performed (close, open, archive, etc.).
    pub action: String,
    /// Human-readable summary.
    pub summary: String,
    /// Unix timestamp (ms) when the action was performed.
    pub created_at: f64,
}
