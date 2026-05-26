//! Browser-backed integration smoke test.
//!
//! Uses the shared Chrome instance from `common::shared_browser()` so the
//! bootstrap cost is paid only once across all integration test files.

mod common;

use common::*;
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

    let tabs = b.run_query("query { tabs(limit: 20) { total items { tabId } } }");
    assert!(
        response_data(&tabs)["tabs"]["total"].as_i64().unwrap_or(0) >= 1,
        "tabs query should report at least one tab: {tabs}"
    );

    let test_group = format!("TEST-Rust-Integration-{}", now_ms());
    let test_group_gql = gql_string(&test_group);

    let open = b.run_query(&format!(
        r#"mutation {{ openTabs(urls: ["https://example.com", "https://example.org"], group: {test_group_gql}, newWindow: true) {{ windowId groupId tabs {{ tabId url groupId groupTitle }} skippedUrls {{ url reason }} }} }}"#
    ));
    assert_ok("openTabs", &open);
    let open_data = &response_data(&open)["openTabs"];
    let window_id = open_data["windowId"]
        .as_i64()
        .expect("openTabs should return windowId");
    let group_id = open_data["groupId"]
        .as_i64()
        .expect("openTabs should return groupId");
    sleep(Duration::from_secs(2));

    let open_reuse = b.run_query(&format!(
        r#"mutation {{ openTabs(windowId: {window_id}, group: {test_group_gql}, urls: ["https://example.org", "https://example.net"]) {{ windowId groupId tabs {{ tabId url groupId groupTitle }} skippedUrls {{ url reason }} }} }}"#
    ));
    assert_ok("openTabs reuse", &open_reuse);
    let reuse_data = &response_data(&open_reuse)["openTabs"];
    assert_eq!(
        reuse_data["tabs"].as_array().map(|tabs| tabs.len()),
        Some(1),
        "expected one newly opened tab after duplicate filtering: {open_reuse}"
    );
    assert_eq!(
        reuse_data["skippedUrls"].as_array().map(|tabs| tabs.len()),
        Some(1),
        "expected one skipped duplicate URL: {open_reuse}"
    );

    let window = b.run_query(&format!(
        "query {{ window(id: {window_id}) {{ windowId tabs {{ tabId url groupId groupTitle }} groups {{ groupId title }} }} }}"
    ));
    assert_ok("window query", &window);
    let window_data = &response_data(&window)["window"];
    assert!(
        window_data["groups"]
            .as_array()
            .is_some_and(|groups| groups.iter().any(|group| {
                group["groupId"].as_i64() == Some(group_id)
                    && group["title"].as_str() == Some(test_group.as_str())
            })),
        "expected created group in window query: {window}"
    );
    assert!(
        window_data["tabs"]
            .as_array()
            .is_some_and(|tabs| tabs.iter().any(|tab| {
                tab["groupId"].as_i64() == Some(group_id)
                    && tab["groupTitle"].as_str() == Some(test_group.as_str())
            })),
        "expected reused openTabs result to stay grouped in the window snapshot: {window}"
    );

    let extra = b.run_query(&format!(
        r#"mutation {{ openTabs(windowId: {window_id}, urls: ["https://example.edu/?ungrouped"]) {{ tabs {{ tabId url groupId }} }} }}"#
    ));
    assert_ok("openTabs extra", &extra);
    let extra_tab_id = response_data(&extra)["openTabs"]["tabs"][0]["tabId"]
        .as_i64()
        .expect("extra openTabs should return tabId");

    let close = b.run_query(&format!(
        "mutation {{ closeTabs(tabIds: [{extra_tab_id}], confirm: true) {{ txid closedTabs }} }}"
    ));
    assert_ok("closeTabs", &close);
    let close_txid = response_data(&close)["closeTabs"]["txid"]
        .as_str()
        .expect("closeTabs should return txid")
        .to_string();
    assert_eq!(
        response_data(&close)["closeTabs"]["closedTabs"].as_i64(),
        Some(1),
        "expected closeTabs to close the extra tab: {close}"
    );

    let after_close = b.run_query(&format!(
        "query {{ window(id: {window_id}) {{ tabs {{ url groupId }} }} }}"
    ));
    assert_ok("window after close", &after_close);
    assert!(
        response_data(&after_close)["window"]["tabs"]
            .as_array()
            .is_some_and(|tabs| {
                tabs.iter()
                    .all(|tab| tab["url"].as_str() != Some("https://example.edu/?ungrouped"))
            }),
        "expected closeTabs to remove the extra tab from the window: {after_close}"
    );

    let history = b.run_query("query { history(limit: 5) { txid action } }");
    assert_ok("history query", &history);
    assert!(
        response_data(&history)["history"]
            .as_array()
            .is_some_and(|items| items.iter().any(|entry| {
                entry["txid"].as_str() == Some(close_txid.as_str())
                    && entry["action"].as_str() == Some("close")
            })),
        "expected closeTabs txid in history results: {history}"
    );

    b.close_test_window(window_id);
}

#[cfg(windows)]
#[test]
#[ignore = "requires built dist artifacts and Chrome"]
fn named_pipe_graphql_open_tabs_existing_window_returns_on_windows() {
    let b = shared_browser();
    let (window_id, _) = b.create_test_window(&["https://example.com"], None);

    let result = run_tabctl_json_with_timeout(
        &b.tabctl_bin,
        &b.root,
        &b.profile_name,
        &b.config_home,
        &b.state_home,
        &["query", &format!(
            "mutation {{ openTabs(windowId: {window_id}, urls: [\"https://example.org\"]) {{ windowId tabs {{ tabId url }} }} }}"
        )],
        Duration::from_secs(20),
    )
    .unwrap_or_else(|e| panic!("named-pipe GraphQL openTabs failed: {e}"));

    assert_ok("named-pipe openTabs", &result);
    let open = &response_data(&result)["openTabs"];
    let tabs = open["tabs"].as_array().expect("openTabs tabs array");
    assert_eq!(tabs.len(), 1, "expected one opened tab: {result}");
    assert_eq!(tabs[0]["url"], "https://example.org/");
    assert_eq!(open["windowId"].as_i64(), Some(window_id));

    b.close_test_window(window_id);
}
