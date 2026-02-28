//! Browser-backed integration tests for features not covered by the original
//! `browser_integration.rs` harness.
//!
//! Uses a shared Chrome instance (initialized once via `shared_browser()`) so each test function
//! doesn't pay the ~30 s bootstrap cost. Run with `--test-threads=1`.

mod common;

use common::*;
use serde_json::Value;
use std::thread::sleep;
use std::time::Duration;

// ── Group operations ────────────────────────────────────────────────────────

#[test]
#[ignore = "requires built dist artifacts and Chrome"]
fn test_group_operations() {
    let b = shared_browser();
    let ts = now_ms();
    let group_name = format!("TEST-GroupOps-{ts}");

    // Create test window with a group
    let (win_id, _tab_ids) = b.create_test_window(
        &["https://example.com", "https://example.org"],
        Some(&group_name),
    );
    let win_str = win_id.to_string();
    sleep(Duration::from_secs(2));

    // ── group-update: rename + color + collapse ──
    let new_title = format!("TEST-Renamed-{ts}");
    let update = b.run(&[
        "group-update",
        "--window",
        &win_str,
        "--group",
        &group_name,
        "--title",
        &new_title,
        "--color",
        "red",
        "--collapsed",
    ]);
    assert_ok("group-update", &update);
    let update_data = response_data(&update);
    assert!(
        update_data.pointer("/summary").is_some(),
        "group-update should return summary: {update}"
    );

    // Verify the update took effect
    let groups = b.run(&["group-list", "--window", &win_str]);
    assert_ok("group-list after update", &groups);
    let groups_arr = response_data(&groups)
        .pointer("/groups")
        .and_then(Value::as_array)
        .expect("group-list should have groups array");
    let found = groups_arr.iter().any(|g| {
        g.get("title").and_then(Value::as_str) == Some(&new_title)
            && g.get("color").and_then(Value::as_str) == Some("red")
    });
    assert!(
        found,
        "renamed group with color=red not found in group-list: {groups}"
    );

    // Undo group-update
    let undo = b.run(&["undo", "--latest"]);
    assert_ok("undo group-update", &undo);
    sleep(Duration::from_secs(1));

    // ── group-assign: open ungrouped tab, assign it ──
    let assign_url = format!("https://example.net/?assign={ts}");
    let open_tab = b.run(&["open", "--window", &win_str, "--url", &assign_url]);
    assert_ok("open ungrouped tab", &open_tab);
    let new_tab_id = response_data(&open_tab)
        .pointer("/createdTabIds/0")
        .and_then(Value::as_i64)
        .expect("open should return createdTabIds");
    let tab_str = new_tab_id.to_string();
    sleep(Duration::from_secs(1));

    let assign = b.run(&[
        "group-assign",
        "--tab",
        &tab_str,
        "--group",
        &group_name,
        "--window",
        &win_str,
    ]);
    assert_ok("group-assign", &assign);
    let assign_data = response_data(&assign);
    assert!(
        assign_data.pointer("/groupId").is_some(),
        "group-assign should return groupId: {assign}"
    );

    // Undo group-assign
    let undo = b.run(&["undo", "--latest"]);
    assert_ok("undo group-assign", &undo);
    sleep(Duration::from_secs(1));

    // ── group-ungroup ──
    let ungroup = b.run(&[
        "group-ungroup",
        "--group",
        &group_name,
        "--window",
        &win_str,
    ]);
    assert_ok("group-ungroup", &ungroup);

    // Verify: group should no longer exist
    let groups_after = b.run(&["group-list", "--window", &win_str]);
    assert_ok("group-list after ungroup", &groups_after);
    let still_exists = response_data(&groups_after)
        .pointer("/groups")
        .and_then(Value::as_array)
        .is_some_and(|groups_arr2| {
            groups_arr2
                .iter()
                .any(|g| g.get("title").and_then(Value::as_str) == Some(&group_name))
        });
    assert!(
        !still_exists,
        "group should be removed after ungroup: {groups_after}"
    );

    // Undo group-ungroup
    let undo = b.run(&["undo", "--latest"]);
    assert_ok("undo group-ungroup", &undo);
    sleep(Duration::from_secs(1));

    // ── group-gather: create duplicate groups and merge ──
    let gather_group = format!("TEST-Gather-{ts}");
    let open_g1 = b.run(&[
        "open",
        "--window",
        &win_str,
        "--url",
        "https://example.com/?g1",
        "--group",
        &gather_group,
    ]);
    assert_ok("open for gather group 1", &open_g1);
    sleep(Duration::from_secs(1));

    // Create a second group with the same name via --create
    let open_g2 = b.run(&[
        "open",
        "--window",
        &win_str,
        "--url",
        "https://example.org/?g2",
    ]);
    assert_ok("open ungrouped for gather", &open_g2);
    let g2_tab_id = response_data(&open_g2)
        .pointer("/createdTabIds/0")
        .and_then(Value::as_i64)
        .expect("open should return createdTabIds for gather");
    let g2_tab_str = g2_tab_id.to_string();
    let assign_g2 = b.run(&[
        "group-assign",
        "--tab",
        &g2_tab_str,
        "--group",
        &gather_group,
        "--create",
        "--window",
        &win_str,
    ]);
    assert_ok("assign for gather (second group)", &assign_g2);
    sleep(Duration::from_secs(1));

    // Gather
    let gather = b.run(&[
        "group-gather",
        "--group",
        &gather_group,
        "--window",
        &win_str,
    ]);
    assert_ok("group-gather", &gather);
    let gather_data = response_data(&gather);
    assert!(
        gather_data.pointer("/summary").is_some(),
        "group-gather should return summary: {gather}"
    );

    // Undo gather
    let undo = b.run(&["undo", "--latest"]);
    assert_ok("undo group-gather", &undo);

    // Cleanup
    b.close_test_window(win_id);
}

// ── Tab operations ──────────────────────────────────────────────────────────

#[test]
#[ignore = "requires built dist artifacts and Chrome"]
fn test_tab_operations() {
    let b = shared_browser();
    let ts = now_ms();
    let group_name = format!("TEST-TabOps-{ts}");

    let (win_id, tab_ids) = b.create_test_window(
        &[
            "https://example.com",
            "https://example.org",
            "https://example.net",
        ],
        Some(&group_name),
    );
    let win_str = win_id.to_string();
    sleep(Duration::from_secs(2));

    assert!(
        tab_ids.len() >= 2,
        "expected at least 2 tabs, got {}",
        tab_ids.len()
    );

    // ── focus ──
    let first_tab_str = tab_ids[0].to_string();
    let focus = b.run(&["focus", "--tab", &first_tab_str]);
    assert_ok("focus", &focus);

    // ── refresh ──
    let refresh = b.run(&["refresh", "--tab", &first_tab_str]);
    assert_ok("refresh", &refresh);

    // ── move-tab: move first tab after the last tab ──
    let last_tab_str = tab_ids.last().unwrap().to_string();
    let move_tab = b.run(&[
        "move-tab",
        "--tab",
        &first_tab_str,
        "--after-tab",
        &last_tab_str,
    ]);
    assert_ok("move-tab", &move_tab);
    let move_data = response_data(&move_tab);
    assert!(
        move_data.pointer("/summary").is_some(),
        "move-tab should return summary: {move_tab}"
    );

    // Verify tab order changed
    let list = b.run(&["list", "--window", &win_str]);
    assert_ok("list after move-tab", &list);
    let windows = response_data(&list)
        .pointer("/windows")
        .and_then(Value::as_array)
        .expect("list should have windows");
    let test_win = windows
        .iter()
        .find(|w| w.get("windowId").and_then(Value::as_i64) == Some(win_id))
        .expect("test window should exist in list");
    let tabs_after = test_win
        .get("tabs")
        .and_then(Value::as_array)
        .expect("test window should have tabs");
    let moved_tab_found = tabs_after
        .iter()
        .any(|t| t.get("tabId").and_then(Value::as_i64) == Some(tab_ids[0]));
    assert!(
        moved_tab_found,
        "moved tab should still exist in test window: {list}"
    );

    // Undo move-tab
    let undo = b.run(&["undo", "--latest"]);
    assert_ok("undo move-tab", &undo);

    // Cleanup
    b.close_test_window(win_id);
}

// ── Window operations ───────────────────────────────────────────────────────

#[test]
#[ignore = "requires built dist artifacts and Chrome"]
fn test_window_operations() {
    let b = shared_browser();
    let ts = now_ms();

    // Create two windows for merge
    let group1 = format!("TEST-MergeSrc-{ts}");
    let group2 = format!("TEST-MergeDst-{ts}");
    let (win1_id, _) = b.create_test_window(
        &["https://example.com", "https://example.org"],
        Some(&group1),
    );
    let (win2_id, _) = b.create_test_window(&["https://example.net"], Some(&group2));
    let win1_str = win1_id.to_string();
    let win2_str = win2_id.to_string();
    sleep(Duration::from_secs(2));

    // ── merge-window ──
    let merge = b.run(&[
        "merge-window",
        "--from",
        &win1_str,
        "--to",
        &win2_str,
        "--confirm",
    ]);
    assert_ok("merge-window", &merge);
    let merge_data = response_data(&merge);
    assert!(
        merge_data
            .pointer("/summary/movedTabs")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            > 0,
        "merge-window should move tabs: {merge}"
    );

    // Undo merge
    let undo = b.run(&["undo", "--latest"]);
    assert_ok("undo merge-window", &undo);
    sleep(Duration::from_secs(2));

    // Cleanup the merge windows
    b.close_test_window(win1_id);
    b.close_test_window(win2_id);

    // ── archive ──
    let archive_group = format!("TEST-Archive-{ts}");
    let (archive_win_id, _) = b.create_test_window(
        &["https://example.com", "https://example.org"],
        Some(&archive_group),
    );
    let archive_win_str = archive_win_id.to_string();
    sleep(Duration::from_secs(2));

    let archive = b.run(&["archive", "--window", &archive_win_str]);
    assert_ok("archive", &archive);
    let archive_data = response_data(&archive);
    assert!(
        archive_data
            .pointer("/summary/movedTabs")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            > 0,
        "archive should move tabs: {archive}"
    );

    // Undo archive
    let undo = b.run(&["undo", "--latest"]);
    assert_ok("undo archive", &undo);
    sleep(Duration::from_secs(2));

    // Cleanup
    b.close_test_window(archive_win_id);
}

// ── Analysis & inspection ───────────────────────────────────────────────────

#[test]
#[ignore = "requires built dist artifacts and Chrome"]
fn test_analysis_and_inspection() {
    let b = shared_browser();
    let ts = now_ms();
    let group_name = format!("TEST-Analysis-{ts}");

    // Create window with test tabs
    let (win_id, _) = b.create_test_window(
        &["https://example.com", "https://example.org"],
        Some(&group_name),
    );
    let win_str = win_id.to_string();
    sleep(Duration::from_secs(2));

    // Open a duplicate (--allow-duplicates bypasses dedup in same group)
    let _ = b.run(&[
        "open",
        "--window",
        &win_str,
        "--url",
        "https://example.com",
        "--allow-duplicates",
    ]);
    sleep(Duration::from_secs(1));

    // ── analyze ──
    let analyze = b.run(&["analyze", "--window", &win_str]);
    assert_ok("analyze", &analyze);

    // ── dedupe (preview only — no --confirm) ──
    let dedupe_preview = b.run(&["dedupe", "--window", &win_str]);
    assert_ok("dedupe preview", &dedupe_preview);

    // ── dedupe (execute) ──
    let dedupe = b.run(&["dedupe", "--window", &win_str, "--confirm"]);
    assert_ok("dedupe confirm", &dedupe);

    // ── history ──
    let history = b.run(&["history", "--limit", "5"]);
    assert_ok("history", &history);
    // history returns a top-level array
    let history_arr = response_data(&history);
    assert!(
        history_arr.is_array() || history_arr.pointer("/entries").is_some(),
        "history should return an array or entries: {history}"
    );

    // ── inspect ──
    let list = b.run(&["list", "--window", &win_str]);
    assert_ok("list for inspect", &list);
    let tabs = response_data(&list)
        .pointer("/windows/0/tabs")
        .and_then(Value::as_array);
    if let Some(tabs) = tabs {
        if let Some(tab_id) = tabs
            .first()
            .and_then(|t| t.get("tabId").and_then(Value::as_i64))
        {
            let tab_str = tab_id.to_string();

            // inspect with page-meta signal
            let inspect = b.run(&["inspect", "--tab", &tab_str, "--signal", "page-meta"]);
            assert_ok("inspect page-meta", &inspect);

            // ── report ──
            let report_json = b.run(&["report", "--window", &win_str, "--format", "json"]);
            assert_ok("report json", &report_json);

            // ── screenshot ──
            let screenshot = b.run(&["screenshot", "--tab", &tab_str, "--mode", "viewport"]);
            assert_ok("screenshot viewport", &screenshot);
        }
    }

    // Cleanup
    b.close_test_window(win_id);
}

// ── Extension reload ────────────────────────────────────────────────────────

#[test]
#[ignore = "requires built dist artifacts and Chrome"]
fn test_extension_reload() {
    let b = shared_browser();

    let reload = b.run(&["reload"]);
    assert_ok("reload", &reload);

    // Wait for extension service worker to restart
    sleep(Duration::from_secs(5));

    // Verify extension is still responsive
    let ping = b.run_timeout(&["ping"], Duration::from_secs(45));
    assert!(
        ping.is_ok(),
        "ping after reload should succeed: {:?}",
        ping.err()
    );
    assert_ok("ping after reload", &ping.unwrap());
}
