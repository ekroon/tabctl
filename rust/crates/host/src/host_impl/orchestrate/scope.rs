use serde_json::Value;

use super::resolve::{resolve_group, resolve_window_id};

/// Tab selection by scope parameters.
///
/// Reimplements `selectTabsByScope` from background.ts as pure logic over
/// snapshot data. Used by close, report, inspect, screenshot, etc.
///
/// A tab selected by scope, with all fields from the snapshot.
#[derive(Debug, Clone)]
pub(crate) struct ScopedTab {
    pub(crate) tab_id: i64,
    pub(crate) window_id: i64,
    pub(crate) index: Option<i64>,
    pub(crate) url: Option<String>,
    pub(crate) title: Option<String>,
    pub(crate) active: Option<bool>,
    pub(crate) pinned: Option<bool>,
    pub(crate) group_id: i64,
    pub(crate) group_title: Option<String>,
    pub(crate) group_color: Option<String>,
    pub(crate) group_collapsed: Option<bool>,
    pub(crate) last_accessed_at: Option<i64>,
}

impl ScopedTab {
    fn from_value(tab: &Value, window_id: i64) -> Option<Self> {
        Some(Self {
            tab_id: tab.get("tabId").and_then(Value::as_i64)?,
            window_id,
            index: tab.get("index").and_then(Value::as_i64),
            url: tab.get("url").and_then(Value::as_str).map(String::from),
            title: tab.get("title").and_then(Value::as_str).map(String::from),
            active: tab.get("active").and_then(Value::as_bool),
            pinned: tab.get("pinned").and_then(Value::as_bool),
            group_id: tab.get("groupId").and_then(Value::as_i64).unwrap_or(-1),
            group_title: tab
                .get("groupTitle")
                .and_then(Value::as_str)
                .map(String::from),
            group_color: tab
                .get("groupColor")
                .and_then(Value::as_str)
                .map(String::from),
            group_collapsed: tab.get("groupCollapsed").and_then(Value::as_bool),
            last_accessed_at: tab.get("lastAccessedAt").and_then(Value::as_i64),
        })
    }
}

pub(crate) struct ScopeResult {
    pub(crate) tabs: Vec<ScopedTab>,
    pub(crate) error: Option<String>,
}

/// Select tabs from snapshot by scope parameters.
///
/// Priority: tabIds > groupId > groupTitle > windowId > all > focused window.
pub(crate) fn select_tabs_by_scope(snapshot: &Value, params: &Value) -> ScopeResult {
    let all_tabs = flatten_tabs(snapshot);

    // By explicit tabIds
    if let Some(tab_ids) = params.get("tabIds").and_then(Value::as_array) {
        let id_set: std::collections::HashSet<i64> =
            tab_ids.iter().filter_map(Value::as_i64).collect();
        if !id_set.is_empty() {
            return ScopeResult {
                tabs: all_tabs
                    .into_iter()
                    .filter(|t| id_set.contains(&t.tab_id))
                    .collect(),
                error: None,
            };
        }
    }

    // By groupId
    if let Some(group_id) = params.get("groupId").and_then(Value::as_i64) {
        return ScopeResult {
            tabs: all_tabs
                .into_iter()
                .filter(|t| t.group_id == group_id)
                .collect(),
            error: None,
        };
    }

    // By groupTitle
    if let Some(group_title) = params.get("groupTitle").and_then(Value::as_str) {
        let window_id_param = params
            .get("windowId")
            .and_then(|v| resolve_window_id(snapshot, v));

        match resolve_group(snapshot, None, Some(group_title), window_id_param) {
            Ok(m) => {
                return ScopeResult {
                    tabs: all_tabs
                        .into_iter()
                        .filter(|t| t.group_id == m.group_id && t.window_id == m.window_id)
                        .collect(),
                    error: None,
                };
            }
            Err(super::OrchStep::Error { message, .. }) => {
                return ScopeResult {
                    tabs: Vec::new(),
                    error: Some(message),
                };
            }
            _ => {}
        }
    }

    // By windowId
    if let Some(raw_win_id) = params.get("windowId") {
        let window_id = resolve_window_id(snapshot, raw_win_id);
        return ScopeResult {
            tabs: match window_id {
                Some(wid) => all_tabs
                    .into_iter()
                    .filter(|t| t.window_id == wid)
                    .collect(),
                None => Vec::new(),
            },
            error: None,
        };
    }

    // Explicit all
    if params.get("all").and_then(Value::as_bool) == Some(true) {
        return ScopeResult {
            tabs: all_tabs,
            error: None,
        };
    }

    // Default: focused window
    let focused_window_id = snapshot
        .get("windows")
        .and_then(Value::as_array)
        .and_then(|wins| {
            wins.iter().find_map(|w| {
                if w.get("focused").and_then(Value::as_bool) == Some(true) {
                    w.get("windowId").and_then(Value::as_i64)
                } else {
                    None
                }
            })
        });

    ScopeResult {
        tabs: match focused_window_id {
            Some(wid) => all_tabs
                .into_iter()
                .filter(|t| t.window_id == wid)
                .collect(),
            None => Vec::new(),
        },
        error: None,
    }
}

/// Flatten all tabs from all windows in the snapshot.
fn flatten_tabs(snapshot: &Value) -> Vec<ScopedTab> {
    let Some(windows) = snapshot.get("windows").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut tabs = Vec::new();
    for win in windows {
        let win_id = win.get("windowId").and_then(Value::as_i64).unwrap_or(0);
        if let Some(win_tabs) = win.get("tabs").and_then(Value::as_array) {
            for tab in win_tabs {
                if let Some(st) = ScopedTab::from_value(tab, win_id) {
                    tabs.push(st);
                }
            }
        }
    }
    tabs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot() -> Value {
        serde_json::json!({
            "windows": [
                {
                    "windowId": 100,
                    "focused": true,
                    "tabs": [
                        {"tabId": 1, "windowId": 100, "index": 0, "url": "https://a.com", "title": "A", "active": true, "pinned": false, "groupId": 10, "groupTitle": "Dev", "groupColor": "blue", "groupCollapsed": false},
                        {"tabId": 2, "windowId": 100, "index": 1, "url": "https://b.com", "title": "B", "active": false, "pinned": false, "groupId": -1}
                    ],
                    "groups": [{"groupId": 10, "title": "Dev", "color": "blue", "collapsed": false}]
                },
                {
                    "windowId": 200,
                    "focused": false,
                    "tabs": [
                        {"tabId": 3, "windowId": 200, "index": 0, "url": "https://c.com", "title": "C", "active": true, "pinned": false, "groupId": -1}
                    ],
                    "groups": []
                }
            ]
        })
    }

    #[test]
    fn select_by_tab_ids() {
        let r = select_tabs_by_scope(&snapshot(), &serde_json::json!({"tabIds": [2, 3]}));
        assert_eq!(r.tabs.len(), 2);
        assert!(r.error.is_none());
    }

    #[test]
    fn select_by_group_id() {
        let r = select_tabs_by_scope(&snapshot(), &serde_json::json!({"groupId": 10}));
        assert_eq!(r.tabs.len(), 1);
        assert_eq!(r.tabs[0].tab_id, 1);
    }

    #[test]
    fn select_by_group_title() {
        let r = select_tabs_by_scope(&snapshot(), &serde_json::json!({"groupTitle": "Dev"}));
        assert_eq!(r.tabs.len(), 1);
        assert_eq!(r.tabs[0].tab_id, 1);
    }

    #[test]
    fn select_by_window_id() {
        let r = select_tabs_by_scope(&snapshot(), &serde_json::json!({"windowId": 200}));
        assert_eq!(r.tabs.len(), 1);
        assert_eq!(r.tabs[0].tab_id, 3);
    }

    #[test]
    fn select_all() {
        let r = select_tabs_by_scope(&snapshot(), &serde_json::json!({"all": true}));
        assert_eq!(r.tabs.len(), 3);
    }

    #[test]
    fn default_selects_focused_window() {
        let r = select_tabs_by_scope(&snapshot(), &serde_json::json!({}));
        assert_eq!(r.tabs.len(), 2);
        assert!(r.tabs.iter().all(|t| t.window_id == 100));
    }
}
