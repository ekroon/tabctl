use serde_json::Value;

use crate::types::{Group, Tab, Window};

/// Convert a snapshot JSON into typed Window objects.
pub(crate) fn windows_from_snapshot(snapshot: &Value) -> Vec<Window> {
    let Some(windows) = snapshot.get("windows").and_then(Value::as_array) else {
        return Vec::new();
    };
    windows.iter().filter_map(window_from_value).collect()
}

fn window_from_value(val: &Value) -> Option<Window> {
    let window_id = val.get("windowId").and_then(Value::as_i64)? as i32;
    let focused = val.get("focused").and_then(Value::as_bool).unwrap_or(false);

    let tabs: Vec<Tab> = val
        .get("tabs")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|t| tab_from_value(t, window_id))
                .collect()
        })
        .unwrap_or_default();

    let groups: Vec<Group> = val
        .get("groups")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|g| group_from_value(g, &tabs))
                .collect()
        })
        .unwrap_or_default();

    let tab_count = tabs.len() as i32;

    Some(Window {
        window_id,
        focused,
        tabs,
        groups,
        tab_count,
    })
}

fn tab_from_value(val: &Value, window_id: i32) -> Option<Tab> {
    Some(Tab {
        tab_id: val.get("tabId").and_then(Value::as_i64)? as i32,
        window_id,
        url: val
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        title: val
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        active: val.get("active").and_then(Value::as_bool).unwrap_or(false),
        group_id: val.get("groupId").and_then(Value::as_i64).unwrap_or(-1) as i32,
        group_title: val
            .get("groupTitle")
            .and_then(Value::as_str)
            .map(String::from),
        pinned: val.get("pinned").and_then(Value::as_bool).unwrap_or(false),
        index: val.get("index").and_then(Value::as_i64).unwrap_or(0) as i32,
    })
}

fn group_from_value(val: &Value, tabs: &[Tab]) -> Option<Group> {
    let group_id = val.get("groupId").and_then(Value::as_i64)? as i32;
    let tab_count = tabs.iter().filter(|t| t.group_id == group_id).count() as i32;
    Some(Group {
        group_id,
        title: val
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        color: val
            .get("color")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        collapsed: val
            .get("collapsed")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        tab_count,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_snapshot() -> Value {
        serde_json::json!({
            "windows": [{
                "windowId": 100,
                "focused": true,
                "tabs": [
                    {"tabId": 1, "windowId": 100, "index": 0, "url": "https://a.com", "title": "A", "active": true, "pinned": false, "groupId": 10, "groupTitle": "Work"},
                    {"tabId": 2, "windowId": 100, "index": 1, "url": "https://b.com", "title": "B", "active": false, "pinned": true, "groupId": -1}
                ],
                "groups": [{"groupId": 10, "title": "Work", "color": "blue", "collapsed": false}]
            }]
        })
    }

    #[test]
    fn converts_snapshot_to_windows() {
        let windows = windows_from_snapshot(&sample_snapshot());
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].window_id, 100);
        assert!(windows[0].focused);
        assert_eq!(windows[0].tabs.len(), 2);
        assert_eq!(windows[0].groups.len(), 1);
        assert_eq!(windows[0].tab_count, 2);
    }

    #[test]
    fn converts_tab_fields() {
        let windows = windows_from_snapshot(&sample_snapshot());
        let tab = &windows[0].tabs[0];
        assert_eq!(tab.tab_id, 1);
        assert_eq!(tab.url, "https://a.com");
        assert_eq!(tab.title, "A");
        assert!(tab.active);
        assert!(!tab.pinned);
        assert_eq!(tab.group_id, 10);
        assert_eq!(tab.group_title.as_deref(), Some("Work"));
    }

    #[test]
    fn group_tab_count_calculated() {
        let windows = windows_from_snapshot(&sample_snapshot());
        let group = &windows[0].groups[0];
        assert_eq!(group.group_id, 10);
        assert_eq!(group.tab_count, 1);
    }
}
