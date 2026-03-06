use juniper::{GraphQLInputObject, GraphQLObject};

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
    /// Unix timestamp (ms) of the last time this tab was accessed, or null if unknown.
    pub last_accessed_at: Option<f64>,
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

/// A URL that was skipped during an openTabs mutation.
#[derive(Debug, Clone, GraphQLObject)]
pub(crate) struct SkippedUrl {
    /// The URL that was not opened.
    pub url: String,
    /// Why it was skipped (e.g. "duplicate", "create_failed").
    pub reason: String,
}

/// Result of an openTabs mutation.
#[derive(Debug, Clone, GraphQLObject)]
pub(crate) struct OpenResult {
    /// The newly created tabs.
    pub tabs: Vec<Tab>,
    /// URLs that were skipped (e.g. deduplicated or failed to create).
    pub skipped_urls: Vec<SkippedUrl>,
    /// Identifier of the window the tabs were opened in.
    pub window_id: Option<i32>,
    /// Identifier of the group the tabs were assigned to, if any.
    pub group_id: Option<i32>,
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

/// Result of a moveGroup mutation.
#[derive(Debug, Clone, GraphQLObject)]
pub(crate) struct MoveGroupResult {
    /// Identifier of the moved group after the operation.
    pub group_id: i32,
    /// Original source window identifier.
    pub window_id: i32,
    /// Destination window identifier.
    pub moved_to_window_id: i32,
    /// Newly created group identifier when regrouping was required.
    pub new_group_id: Option<i32>,
    /// Number of tabs moved with the group.
    pub moved_tabs: i32,
}

/// Result of a mergeWindows mutation.
#[derive(Debug, Clone, GraphQLObject)]
pub(crate) struct MergeWindowsResult {
    /// Source window identifier.
    pub from_window_id: i32,
    /// Destination window identifier.
    pub to_window_id: i32,
    /// Whether the source window was closed after the move.
    pub source_closed: bool,
    /// Number of tabs moved.
    pub moved_tabs: i32,
    /// Number of grouped batches moved.
    pub moved_groups: i32,
}

/// A single duplicate-group merge operation from gatherGroups.
#[derive(Debug, Clone, GraphQLObject)]
pub(crate) struct GatheredGroup {
    /// Window containing the merged groups.
    pub window_id: i32,
    /// Title shared by the duplicate groups.
    pub group_title: String,
    /// Group ID kept as the primary destination.
    pub primary_group_id: i32,
    /// Number of duplicate groups merged into the primary one.
    pub merged_group_count: i32,
    /// Number of tabs moved into the primary group.
    pub moved_tabs: i32,
}

/// Summary counts for gatherGroups.
#[derive(Debug, Clone, GraphQLObject)]
pub(crate) struct GatherSummary {
    /// Number of duplicate groups merged.
    pub merged_groups: i32,
    /// Number of tabs moved during the gather.
    pub moved_tabs: i32,
}

/// Result of a gatherGroups mutation.
#[derive(Debug, Clone, GraphQLObject)]
pub(crate) struct GatherResult {
    /// Per-window merge details.
    pub merged: Vec<GatheredGroup>,
    /// Aggregate counts for the operation.
    pub summary: GatherSummary,
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

/// Result of a deduplicateTabs mutation.
#[derive(Debug, Clone, GraphQLObject)]
pub(crate) struct DedupeResult {
    /// Transaction ID for the close-style undo payload, if tabs were actually closed.
    pub txid: Option<String>,
    /// Number of duplicate tabs closed.
    pub closed_tabs: i32,
    /// Number of duplicate groups detected.
    pub duplicate_groups: i32,
    /// Tabs that would be closed (or were closed) by dedupe.
    pub candidate_tabs: Vec<Tab>,
}

/// Input selector specification for inspectTabs.
#[derive(Debug, Clone, GraphQLInputObject)]
pub(crate) struct SelectorSpecInput {
    /// Result key for this selector.
    pub name: String,
    /// CSS selector to execute in the page.
    pub selector: String,
    /// Attribute or extraction mode (e.g. text, href-url).
    pub attr: Option<String>,
}

/// A named signal payload returned by inspectTabs.
#[derive(Debug, Clone, GraphQLObject)]
pub(crate) struct InspectSignalResult {
    /// Signal name.
    pub name: String,
    /// Raw JSON payload for the signal value.
    pub value_json: String,
}

/// A single inspectTabs result row.
#[derive(Debug, Clone, GraphQLObject)]
pub(crate) struct InspectEntry {
    /// Tab identifier.
    pub tab_id: i32,
    /// Window identifier.
    pub window_id: i32,
    /// Tab URL.
    pub url: String,
    /// Tab title.
    pub title: Option<String>,
    /// Signal payloads keyed by signal name.
    pub signals: Vec<InspectSignalResult>,
}

/// Summary counts for inspectTabs.
#[derive(Debug, Clone, GraphQLObject)]
pub(crate) struct InspectTotals {
    /// Number of tabs inspected.
    pub tabs: i32,
    /// Number of distinct signals requested.
    pub signals: i32,
    /// Number of tab×signal tasks executed.
    pub tasks: i32,
}

/// Result of an inspectTabs query.
#[derive(Debug, Clone, GraphQLObject)]
pub(crate) struct InspectResult {
    /// Summary counts.
    pub totals: InspectTotals,
    /// Per-tab results.
    pub entries: Vec<InspectEntry>,
}

/// A single reportTabs entry.
#[derive(Debug, Clone, GraphQLObject)]
pub(crate) struct ReportEntry {
    /// Tab identifier.
    pub tab_id: i32,
    /// Window identifier.
    pub window_id: i32,
    /// Tab URL.
    pub url: String,
    /// Tab title.
    pub title: Option<String>,
    /// Group identifier.
    pub group_id: i32,
    /// Group title, if any.
    pub group_title: Option<String>,
    /// Group color, if any.
    pub group_color: Option<String>,
    /// Extracted page description.
    pub description: String,
    /// Last-accessed timestamp in ms, if known.
    pub last_accessed_at: Option<f64>,
}

/// Summary counts for reportTabs.
#[derive(Debug, Clone, GraphQLObject)]
pub(crate) struct ReportTotals {
    /// Number of tabs included in the report.
    pub tabs: i32,
}

/// Result of a reportTabs query.
#[derive(Debug, Clone, GraphQLObject)]
pub(crate) struct ReportResult {
    /// Timestamp when the report was generated.
    pub generated_at: f64,
    /// Report entries.
    pub entries: Vec<ReportEntry>,
    /// Summary counts.
    pub totals: ReportTotals,
}

/// A single screenshot tile.
#[derive(Debug, Clone, GraphQLObject)]
pub(crate) struct ScreenshotTile {
    /// Zero-based tile index.
    pub index: i32,
    /// Total number of tiles emitted for the tab.
    pub total: Option<i32>,
    /// Tile origin X coordinate.
    pub x: i32,
    /// Tile origin Y coordinate.
    pub y: i32,
    /// Tile width.
    pub width: i32,
    /// Tile height.
    pub height: i32,
    /// Device pixel ratio used during capture.
    pub scale: f64,
    /// Approximate encoded byte size.
    pub bytes: Option<i32>,
    /// Whether the tile was scaled down.
    pub scaled: Option<bool>,
    /// Whether the tile still exceeded the requested byte limit.
    pub oversized: Option<bool>,
    /// Captured data URL payload.
    pub data_url: Option<String>,
}

/// A screenshot error payload.
#[derive(Debug, Clone, GraphQLObject)]
pub(crate) struct ScreenshotError {
    /// Error message.
    pub message: String,
}

/// A single captureScreenshots entry.
#[derive(Debug, Clone, GraphQLObject)]
pub(crate) struct ScreenshotEntry {
    /// Tab identifier.
    pub tab_id: i32,
    /// Window identifier.
    pub window_id: i32,
    /// Group identifier.
    pub group_id: i32,
    /// Tab URL.
    pub url: String,
    /// Tab title.
    pub title: Option<String>,
    /// Error payload for unsupported URLs, if any.
    pub error: Option<ScreenshotError>,
    /// Captured tiles.
    pub tiles: Vec<ScreenshotTile>,
}

/// Summary counts for captureScreenshots.
#[derive(Debug, Clone, GraphQLObject)]
pub(crate) struct ScreenshotTotals {
    /// Number of tabs included.
    pub tabs: i32,
    /// Number of tiles captured.
    pub tiles: i32,
}

/// Result of a captureScreenshots query.
#[derive(Debug, Clone, GraphQLObject)]
pub(crate) struct ScreenshotResult {
    /// Summary counts.
    pub totals: ScreenshotTotals,
    /// Per-tab capture entries.
    pub entries: Vec<ScreenshotEntry>,
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

/// Result of a reloadExtension mutation.
#[derive(Debug, Clone, GraphQLObject)]
pub(crate) struct ReloadResult {
    /// Whether the extension reported that reload has started.
    pub reloading: bool,
}
