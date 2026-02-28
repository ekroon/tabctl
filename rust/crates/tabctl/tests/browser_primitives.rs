//! Integration tests for extension primitive actions (`p:*` prefix).
//!
//! Each test exercises a single primitive to verify the thin Chrome API wrapper
//! works end-to-end. Uses the shared browser fixture for efficiency.
//! Run with `--test-threads=1`.

mod common;

use common::*;
use serde_json::{json, Value};
use std::thread::sleep;
use std::time::Duration;

/// Helper: send a primitive action via `tabctl raw`.
fn send_primitive(b: &SharedBrowser, action: &str, params: Value) -> Value {
    let full_action = format!("p:{action}");
    let params_str = params.to_string();
    b.run(&["raw", "--action", &full_action, "--params", &params_str])
}

// ── Read primitives ─────────────────────────────────────────────────────────

#[test]
#[ignore = "requires built dist artifacts and Chrome"]
fn primitive_snapshot_returns_windows() {
    let b = shared_browser();
    let r = send_primitive(b, "snapshot", json!({}));
    let data = response_data(&r);
    assert!(
        data.get("windows").and_then(Value::as_array).is_some(),
        "p:snapshot should return windows array: {r}"
    );
    let windows = data["windows"].as_array().unwrap();
    assert!(!windows.is_empty(), "should have at least one window");
    let first_win = &windows[0];
    assert!(
        first_win.get("windowId").is_some(),
        "window should have windowId"
    );
    assert!(
        first_win.get("tabs").and_then(Value::as_array).is_some(),
        "window should have tabs"
    );
}

#[test]
#[ignore = "requires built dist artifacts and Chrome"]
fn primitive_tab_create_get_and_remove() {
    let b = shared_browser();

    // Create a test window to work in
    let (win_id, _) = b.create_test_window(&["https://example.com"], None);
    let _cleanup = TestWindowGuard::new(b, win_id);

    // p:tab-create
    let created = send_primitive(
        b,
        "tab-create",
        json!({"windowId": win_id, "url": "https://example.org", "active": false}),
    );
    let tab = response_data(&created);
    let tab_id = tab
        .get("id")
        .and_then(Value::as_i64)
        .expect("p:tab-create should return tab with id");
    assert_eq!(
        tab.get("windowId").and_then(Value::as_i64),
        Some(win_id),
        "created tab should be in the requested window"
    );

    // p:tab-get
    let got = send_primitive(b, "tab-get", json!({"tabId": tab_id}));
    let got_tab = response_data(&got);
    assert_eq!(
        got_tab.get("id").and_then(Value::as_i64),
        Some(tab_id),
        "p:tab-get should return the same tab"
    );

    // p:tab-remove
    let removed = send_primitive(b, "tab-remove", json!({"tabIds": [tab_id]}));
    let removed_data = response_data(&removed);
    assert_eq!(
        removed_data.get("removed").and_then(Value::as_bool),
        Some(true),
        "p:tab-remove should return removed: true"
    );
}

#[test]
#[ignore = "requires built dist artifacts and Chrome"]
fn primitive_tab_update_and_query() {
    let b = shared_browser();
    let (win_id, tab_ids) =
        b.create_test_window(&["https://example.com", "https://example.org"], None);
    let _cleanup = TestWindowGuard::new(b, win_id);
    sleep(Duration::from_secs(1));

    // p:tab-update — activate the second tab
    let tab_id = tab_ids[1];
    let updated = send_primitive(b, "tab-update", json!({"tabId": tab_id, "active": true}));
    let updated_tab = response_data(&updated);
    assert_eq!(
        updated_tab.get("active").and_then(Value::as_bool),
        Some(true),
        "p:tab-update should return updated tab with active=true"
    );

    // p:tab-query — find active tabs in window
    let queried = send_primitive(
        b,
        "tab-query",
        json!({"query": {"windowId": win_id, "active": true}}),
    );
    let tabs = response_data(&queried)
        .as_array()
        .expect("p:tab-query should return array");
    assert_eq!(tabs.len(), 1, "should find exactly one active tab");
    assert_eq!(
        tabs[0].get("id").and_then(Value::as_i64),
        Some(tab_id),
        "active tab should be the one we updated"
    );
}

#[test]
#[ignore = "requires built dist artifacts and Chrome"]
fn primitive_tab_move() {
    let b = shared_browser();
    let (win_id, tab_ids) = b.create_test_window(
        &[
            "https://example.com",
            "https://example.org",
            "https://example.net",
        ],
        None,
    );
    let _cleanup = TestWindowGuard::new(b, win_id);
    sleep(Duration::from_secs(1));

    // Move the last tab to index 0
    let tab_id = tab_ids[2];
    let moved = send_primitive(
        b,
        "tab-move",
        json!({"tabIds": tab_id, "windowId": win_id, "index": 0}),
    );
    let moved_tab = response_data(&moved);
    assert_eq!(
        moved_tab.get("index").and_then(Value::as_i64),
        Some(0),
        "p:tab-move should return tab at new index"
    );
}

#[test]
#[ignore = "requires built dist artifacts and Chrome"]
fn primitive_tab_reload() {
    let b = shared_browser();
    let (win_id, tab_ids) = b.create_test_window(&["https://example.com"], None);
    let _cleanup = TestWindowGuard::new(b, win_id);

    let reloaded = send_primitive(b, "tab-reload", json!({"tabId": tab_ids[0]}));
    let data = response_data(&reloaded);
    assert_eq!(
        data.get("reloaded").and_then(Value::as_bool),
        Some(true),
        "p:tab-reload should return reloaded: true"
    );
}

// ── Group primitives ────────────────────────────────────────────────────────

#[test]
#[ignore = "requires built dist artifacts and Chrome"]
fn primitive_tab_group_update_and_ungroup() {
    let b = shared_browser();
    let (win_id, tab_ids) =
        b.create_test_window(&["https://example.com", "https://example.org"], None);
    let _cleanup = TestWindowGuard::new(b, win_id);
    sleep(Duration::from_secs(1));

    // p:tab-group — create a new group
    let grouped = send_primitive(
        b,
        "tab-group",
        json!({"tabIds": tab_ids, "createProperties": {"windowId": win_id}}),
    );
    let group_data = response_data(&grouped);
    let group_id = group_data
        .get("groupId")
        .and_then(Value::as_i64)
        .expect("p:tab-group should return groupId");
    assert!(group_id > 0, "groupId should be positive");

    // p:group-update — name and color the group
    let ts = now_ms();
    let title = format!("TEST-Prim-{ts}");
    let updated = send_primitive(
        b,
        "group-update",
        json!({"groupId": group_id, "title": title, "color": "blue"}),
    );
    let updated_group = response_data(&updated);
    assert_eq!(
        updated_group.get("title").and_then(Value::as_str),
        Some(title.as_str()),
        "p:group-update should return group with new title"
    );
    assert_eq!(
        updated_group.get("color").and_then(Value::as_str),
        Some("blue"),
        "p:group-update should return group with new color"
    );

    // p:tab-ungroup — remove tabs from group
    let ungrouped = send_primitive(b, "tab-ungroup", json!({"tabIds": tab_ids}));
    let ug_data = response_data(&ungrouped);
    assert_eq!(
        ug_data.get("ungrouped").and_then(Value::as_bool),
        Some(true),
        "p:tab-ungroup should return ungrouped: true"
    );
}

// ── Window primitives ───────────────────────────────────────────────────────

#[test]
#[ignore = "requires built dist artifacts and Chrome"]
fn primitive_window_create_update_and_remove() {
    let b = shared_browser();

    // p:window-create
    let created = send_primitive(b, "window-create", json!({"focused": false}));
    let win = response_data(&created);
    let win_id = win
        .get("id")
        .and_then(Value::as_i64)
        .expect("p:window-create should return window with id");

    // p:window-update — focus the window
    let updated = send_primitive(
        b,
        "window-update",
        json!({"windowId": win_id, "focused": true}),
    );
    let updated_win = response_data(&updated);
    assert_eq!(
        updated_win.get("id").and_then(Value::as_i64),
        Some(win_id),
        "p:window-update should return the same window"
    );

    // p:window-remove
    let removed = send_primitive(b, "window-remove", json!({"windowId": win_id}));
    let rem_data = response_data(&removed);
    assert_eq!(
        rem_data.get("removed").and_then(Value::as_bool),
        Some(true),
        "p:window-remove should return removed: true"
    );
}

// ── Content primitives ──────────────────────────────────────────────────────

#[test]
#[ignore = "requires built dist artifacts and Chrome"]
fn primitive_execute_script_extract_page_meta() {
    let b = shared_browser();
    let (win_id, tab_ids) = b.create_test_window(&["https://example.com"], None);
    let _cleanup = TestWindowGuard::new(b, win_id);
    sleep(Duration::from_secs(2));

    let result = send_primitive(
        b,
        "execute-script",
        json!({"tabId": tab_ids[0], "func": "extractPageMeta", "args": [500], "timeoutMs": 8000}),
    );
    let data = response_data(&result);
    // extractPageMeta returns {description, h1} or null for non-scriptable
    // example.com should be scriptable
    assert!(
        data.is_object() || data.is_null(),
        "p:execute-script extractPageMeta should return object or null: {result}"
    );
}

// ── Test cleanup helper ─────────────────────────────────────────────────────

struct TestWindowGuard<'a> {
    browser: &'a SharedBrowser,
    window_id: i64,
}

impl<'a> TestWindowGuard<'a> {
    fn new(browser: &'a SharedBrowser, window_id: i64) -> Self {
        Self { browser, window_id }
    }
}

impl<'a> Drop for TestWindowGuard<'a> {
    fn drop(&mut self) {
        self.browser.close_test_window(self.window_id);
    }
}
