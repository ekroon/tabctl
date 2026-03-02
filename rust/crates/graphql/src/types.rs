use juniper::GraphQLObject;

/// A browser tab.
#[derive(Debug, Clone, GraphQLObject)]
pub(crate) struct Tab {
    pub tab_id: i32,
    pub window_id: i32,
    pub url: String,
    pub title: String,
    pub active: bool,
    pub group_id: i32,
    pub group_title: Option<String>,
    pub pinned: bool,
    pub index: i32,
}

/// A browser window.
#[derive(Debug, Clone, GraphQLObject)]
pub(crate) struct Window {
    pub window_id: i32,
    pub focused: bool,
    pub tabs: Vec<Tab>,
    pub groups: Vec<Group>,
    pub tab_count: i32,
}

/// A tab group within a window.
#[derive(Debug, Clone, GraphQLObject)]
pub(crate) struct Group {
    pub group_id: i32,
    pub title: String,
    pub color: String,
    pub collapsed: bool,
    pub tab_count: i32,
}

/// Result of a closeTabs mutation.
#[derive(Debug, Clone, GraphQLObject)]
pub(crate) struct CloseResult {
    pub txid: String,
    pub closed_tabs: i32,
    pub remaining_tabs: Vec<Tab>,
}

/// Result of an openTabs mutation.
#[derive(Debug, Clone, GraphQLObject)]
pub(crate) struct OpenResult {
    pub tabs: Vec<Tab>,
}

/// Result of a refreshTabs mutation.
#[derive(Debug, Clone, GraphQLObject)]
pub(crate) struct RefreshResult {
    pub refreshed_tabs: i32,
}

/// Paginated list of tabs.
#[derive(Debug, Clone, GraphQLObject)]
pub(crate) struct TabPage {
    pub items: Vec<Tab>,
    pub total: i32,
    pub offset: i32,
    pub has_more: bool,
}
