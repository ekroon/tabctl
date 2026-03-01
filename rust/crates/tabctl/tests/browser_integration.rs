//! Browser-backed integration smoke test.
//!
//! Uses the shared Chrome instance from `common::shared_browser()` so the
//! bootstrap cost is paid only once across all integration test files.

mod common;

use common::*;
use serde_json::Value;
use std::thread::sleep;
use std::time::Duration;

#[test]
#[ignore = "requires built dist artifacts and Chrome"]
fn real_browser_integration_harness_passes() {
    let b = shared_browser();

    let ping = b.run(&["ping"]);
    assert_ok("ping", &ping);

    let version = b.run(&["version"]);
    assert_ok("version", &version);

    let list_all = b.run(&["list", "--all"]);
    assert_ok("list --all", &list_all);

    let test_group = format!("TEST-Rust-Integration-{}", now_ms());
    let open = b.run(&[
        "open",
        "--new-window",
        "--url",
        "https://example.com",
        "--url",
        "https://example.org",
        "--group",
        test_group.as_str(),
    ]);
    assert_ok("open", &open);
    let open_window_id = response_data(&open)
        .pointer("/windowId")
        .and_then(Value::as_i64)
        .expect("open payload missing data.windowId");
    let open_window_id_arg = open_window_id.to_string();
    sleep(Duration::from_secs(2));

    let open_reuse_group = b.run(&[
        "open",
        "--window",
        open_window_id_arg.as_str(),
        "--group",
        test_group.as_str(),
        "--color",
        "blue",
        "--url",
        "https://example.com",
        "--url",
        "https://example.net",
    ]);
    assert_ok("open into existing group", &open_reuse_group);
    let open_reuse_data = response_data(&open_reuse_group);
    assert_eq!(
        open_reuse_data
            .pointer("/summary/createdTabs")
            .and_then(Value::as_u64),
        Some(1),
        "expected duplicate URL to be skipped and one tab to be created: {open_reuse_group}"
    );
    assert_eq!(
        open_reuse_data
            .pointer("/summary/skippedUrls")
            .and_then(Value::as_u64),
        Some(1),
        "expected one skipped duplicate URL: {open_reuse_group}"
    );

    sleep(Duration::from_secs(2));

    let list_last_focused = b.run(&["list", "--window", open_window_id_arg.as_str()]);
    assert_ok("list --window <open window>", &list_last_focused);
    let windows = response_data(&list_last_focused)
        .pointer("/windows")
        .and_then(Value::as_array)
        .expect("list payload missing data.windows");
    let mut example_net_group_title: Option<String> = None;
    let mut example_net_ungrouped = false;
    for window in windows {
        if let Some(tabs) = window.get("tabs").and_then(Value::as_array) {
            for tab in tabs {
                let url = tab.get("url").and_then(Value::as_str).unwrap_or_default();
                if url.starts_with("https://example.net") {
                    if tab.get("groupId").and_then(Value::as_i64) == Some(-1) {
                        example_net_ungrouped = true;
                    }
                    example_net_group_title = tab
                        .get("groupTitle")
                        .and_then(Value::as_str)
                        .map(ToOwned::to_owned);
                }
            }
        }
    }
    assert_eq!(
        example_net_group_title.as_deref(),
        Some(test_group.as_str()),
        "example.net tab should be in the target group: {list_last_focused}"
    );
    assert!(
        !example_net_ungrouped,
        "example.net tab must not be left ungrouped: {list_last_focused}"
    );

    let busy_marker = now_ms();
    let anchor_group = format!("TEST-Rust-Anchor-{}", busy_marker);
    let noise_url_a = format!("https://example.edu/?noise={busy_marker}");
    let noise_url_b = format!("https://example.gov/?noise={busy_marker}");
    let anchor_url = format!("https://example.org/?anchor={busy_marker}");
    let move_url_a = format!("https://example.dev/?movea={busy_marker}");
    let move_url_b = format!("https://example.app/?moveb={busy_marker}");

    let open_noise = b.run(&[
        "open",
        "--window",
        open_window_id_arg.as_str(),
        "--url",
        noise_url_a.as_str(),
        "--url",
        noise_url_b.as_str(),
    ]);
    assert_ok("open ungrouped noise tabs", &open_noise);

    let open_anchor_group = b.run(&[
        "open",
        "--window",
        open_window_id_arg.as_str(),
        "--group",
        anchor_group.as_str(),
        "--url",
        anchor_url.as_str(),
    ]);
    assert_ok("open anchor group", &open_anchor_group);

    let open_busy_reuse = b.run(&[
        "open",
        "--window",
        open_window_id_arg.as_str(),
        "--group",
        test_group.as_str(),
        "--url",
        move_url_a.as_str(),
        "--url",
        move_url_b.as_str(),
    ]);
    assert_ok("open into busy existing group", &open_busy_reuse);
    let open_busy_data = response_data(&open_busy_reuse);
    assert_eq!(
        open_busy_data
            .pointer("/summary/createdTabs")
            .and_then(Value::as_u64),
        Some(2),
        "expected two created tabs in busy reuse scenario: {open_busy_reuse}"
    );
    let busy_created_tab_ids: Vec<i64> = open_busy_data
        .pointer("/createdTabIds")
        .and_then(Value::as_array)
        .map(|entries| entries.iter().filter_map(Value::as_i64).collect())
        .or_else(|| {
            open_busy_data
                .pointer("/created")
                .and_then(Value::as_array)
                .map(|entries| {
                    entries
                        .iter()
                        .filter_map(|entry| entry.get("tabId").and_then(Value::as_i64))
                        .collect()
                })
        })
        .expect("open busy reuse payload missing data.createdTabIds/data.created");
    assert_eq!(
        busy_created_tab_ids.len(),
        2,
        "expected two created tab ids in busy reuse scenario: {open_busy_reuse}"
    );

    let assert_move_tabs_grouped = |payload: &Value, phase: &str| {
        let windows = response_data(payload)
            .pointer("/windows")
            .and_then(Value::as_array)
            .expect("list payload missing data.windows");
        let mut found: Vec<(i64, Option<String>, i64)> = Vec::new();
        for window in windows {
            if let Some(tabs) = window.get("tabs").and_then(Value::as_array) {
                for tab in tabs {
                    if let Some(tab_id) = tab.get("tabId").and_then(Value::as_i64) {
                        if busy_created_tab_ids.contains(&tab_id) {
                            found.push((
                                tab.get("groupId").and_then(Value::as_i64).unwrap_or(-1),
                                tab.get("groupTitle")
                                    .and_then(Value::as_str)
                                    .map(ToOwned::to_owned),
                                tab_id,
                            ));
                        }
                    }
                }
            }
        }
        assert_eq!(
            found.len(),
            busy_created_tab_ids.len(),
            "expected to find all created move tabs during {phase}: {payload}"
        );
        for (group_id, group_title, tab_id) in found {
            assert_ne!(
                group_id, -1,
                "tab {tab_id} must remain grouped during {phase}: {payload}"
            );
            assert_eq!(
                group_title.as_deref(),
                Some(test_group.as_str()),
                "tab {tab_id} must stay in group {test_group} during {phase}: {payload}"
            );
        }
    };

    let list_busy_open = b.run(&["list", "--window", open_window_id_arg.as_str()]);
    assert_ok("list busy window after open", &list_busy_open);
    assert_move_tabs_grouped(&list_busy_open, "busy-open-immediate");

    sleep(Duration::from_secs(2));
    let list_busy_open_delayed = b.run(&["list", "--window", open_window_id_arg.as_str()]);
    assert_ok(
        "list busy window after delayed open",
        &list_busy_open_delayed,
    );
    assert_move_tabs_grouped(&list_busy_open_delayed, "busy-open-delayed");

    let move_group = b.run(&[
        "move-group",
        "--window",
        open_window_id_arg.as_str(),
        "--group",
        test_group.as_str(),
        "--after-group",
        anchor_group.as_str(),
    ]);
    assert_ok("move-group", &move_group);

    let list_after_move = b.run(&["list", "--window", open_window_id_arg.as_str()]);
    assert_ok("list after move-group", &list_after_move);
    assert_move_tabs_grouped(&list_after_move, "move-group-immediate");

    sleep(Duration::from_secs(2));
    let list_after_move_delayed = b.run(&["list", "--window", open_window_id_arg.as_str()]);
    assert_ok("list after delayed move-group", &list_after_move_delayed);
    assert_move_tabs_grouped(&list_after_move_delayed, "move-group-delayed");

    let groups = b.run(&["group-list", "--window", open_window_id_arg.as_str()]);
    assert_ok("group-list", &groups);

    let close_group = b.run(&[
        "close",
        "--group",
        test_group.as_str(),
        "--window",
        open_window_id_arg.as_str(),
        "--confirm",
    ]);
    assert_ok("close", &close_group);

    let undo = b.run(&["undo", "--latest"]);
    assert_ok("undo --latest", &undo);
}
