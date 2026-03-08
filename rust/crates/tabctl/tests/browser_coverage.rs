//! Browser-backed GraphQL coverage tests.
//!
//! Uses a shared Chrome instance so the extension bootstrap cost is paid only
//! once. Run with `--test-threads=1`.

mod common;

use common::*;
use serde_json::json;
use serde_json::Value;
use std::thread::sleep;
use std::time::Duration;

fn window_query(b: &SharedBrowser, window_id: i64) -> Value {
    b.run_query(&format!(
        "query {{ window(id: {window_id}) {{ windowId tabs {{ tabId url index active groupId groupTitle }} groups {{ groupId title color collapsed tabCount }} }} }}"
    ))
}

#[test]
#[ignore = "requires built dist artifacts and Chrome"]
fn test_group_and_window_graphql_workflows() {
    let b = shared_browser();
    let ts = now_ms();
    let group_name = format!("TEST-GroupOps-{ts}");

    let (window_id, tab_ids) =
        b.create_test_window(&["https://example.com", "https://example.org"], None);
    sleep(Duration::from_secs(2));

    let initial_group = b.send_host_request(
        "p:tab-group",
        json!({"tabIds": tab_ids, "createProperties": {"windowId": window_id}}),
    );
    let initial_group_id = response_data(&initial_group)["groupId"]
        .as_i64()
        .expect("initial group id");
    b.send_host_request(
        "p:group-update",
        json!({"groupId": initial_group_id, "title": group_name, "color": "blue"}),
    );
    sleep(Duration::from_secs(1));

    let renamed_group = format!("TEST-Renamed-{ts}");
    let renamed_group_gql = gql_string(&renamed_group);
    let update = b.run_query(&format!(
        r#"mutation {{ updateGroup(groupId: {initial_group_id}, title: {renamed_group_gql}, color: "red", collapsed: true) {{ groupId title color collapsed }} }}"#
    ));
    assert_ok("updateGroup", &update);
    assert_eq!(
        response_data(&update)["updateGroup"]["title"].as_str(),
        Some(renamed_group.as_str())
    );

    let verify_update = window_query(b, window_id);
    assert!(
        response_data(&verify_update)["window"]["groups"]
            .as_array()
            .is_some_and(|groups| groups.iter().any(|group| {
                group["groupId"].as_i64() == Some(initial_group_id)
                    && group["title"].as_str() == Some(renamed_group.as_str())
                    && group["color"].as_str() == Some("red")
                    && group["collapsed"].as_bool() == Some(true)
            })),
        "expected updated group metadata: {verify_update}"
    );

    let extra = b.run_query(&format!(
        r#"mutation {{ openTabs(windowId: {window_id}, urls: ["https://example.net/?assign={ts}"]) {{ tabs {{ tabId }} }} }}"#
    ));
    assert_ok("openTabs extra", &extra);
    let extra_tab_id = response_data(&extra)["openTabs"]["tabs"][0]["tabId"]
        .as_i64()
        .expect("extra tab id");

    let assign = b.run_query(&format!(
        "mutation {{ assignToGroup(tabIds: [{extra_tab_id}], groupTitle: {renamed_group_gql}) {{ groupId title }} }}"
    ));
    assert_ok("assignToGroup", &assign);
    assert_eq!(
        response_data(&assign)["assignToGroup"]["groupId"].as_i64(),
        Some(initial_group_id)
    );

    let ungroup = b.run_query(&format!(
        "mutation {{ ungroupTabs(tabIds: [{extra_tab_id}]) {{ tabId groupId }} }}"
    ));
    assert_ok("ungroupTabs", &ungroup);
    sleep(Duration::from_secs(1));
    let ungrouped_tab = b.run_query(&format!(
        "query {{ tab(id: {extra_tab_id}) {{ tabId groupId groupTitle }} }}"
    ));
    assert_eq!(
        response_data(&ungrouped_tab)["tab"]["tabId"].as_i64(),
        Some(extra_tab_id)
    );

    let gather_title = format!("TEST-Gather-{ts}");
    let gather_title_gql = gql_string(&gather_title);
    let dup_a = b.send_host_request(
        "p:tab-create",
        json!({"windowId": window_id, "url": format!("https://example.com/?g1={ts}"), "active": false}),
    );
    let dup_a_id = response_data(&dup_a)["id"]
        .as_i64()
        .expect("first duplicate tab id");
    let group_a = b.send_host_request(
        "p:tab-group",
        json!({"tabIds": [dup_a_id], "createProperties": {"windowId": window_id}}),
    );
    let group_a_id = response_data(&group_a)["groupId"]
        .as_i64()
        .expect("first duplicate group id");
    b.send_host_request(
        "p:group-update",
        json!({"groupId": group_a_id, "title": gather_title, "color": "blue"}),
    );

    let dup_b = b.send_host_request(
        "p:tab-create",
        json!({"windowId": window_id, "url": format!("https://example.org/?g2={ts}"), "active": false}),
    );
    let dup_b_id = response_data(&dup_b)["id"]
        .as_i64()
        .expect("second duplicate tab id");
    let group_b = b.send_host_request(
        "p:tab-group",
        json!({"tabIds": [dup_b_id], "createProperties": {"windowId": window_id}}),
    );
    let group_b_id = response_data(&group_b)["groupId"]
        .as_i64()
        .expect("second duplicate group id");
    b.send_host_request(
        "p:group-update",
        json!({"groupId": group_b_id, "title": gather_title, "color": "green"}),
    );
    sleep(Duration::from_secs(1));

    let gather = b.run_query(&format!(
        "mutation {{ gatherGroups(windowId: {window_id}, groupTitle: {gather_title_gql}) {{ summary {{ mergedGroups movedTabs }} merged {{ primaryGroupId mergedGroupCount movedTabs }} }} }}"
    ));
    assert_ok("gatherGroups", &gather);
    assert!(
        response_data(&gather)["gatherGroups"]["summary"]["mergedGroups"]
            .as_i64()
            .unwrap_or(0)
            >= 1,
        "expected gatherGroups to merge duplicates: {gather}"
    );

    b.close_test_window(window_id);
}

#[test]
#[ignore = "requires built dist artifacts and Chrome"]
fn test_browser_state_history_graphql_workflows() {
    let b = shared_browser();
    let ts = now_ms();
    let group_name = format!("TEST-BrowserState-{ts}");
    let group_name_gql = gql_string(&group_name);

    let (window_id, tab_ids) =
        b.create_test_window(&["https://example.com", "https://example.org"], None);
    sleep(Duration::from_secs(2));

    let initial_group = b.send_host_request(
        "p:tab-group",
        json!({"tabIds": tab_ids, "createProperties": {"windowId": window_id}}),
    );
    let group_id = response_data(&initial_group)["groupId"]
        .as_i64()
        .expect("group id");
    b.send_host_request(
        "p:group-update",
        json!({"groupId": group_id, "title": group_name, "color": "blue"}),
    );

    sleep(Duration::from_secs(2));

    let latest = b.run_query(
        "query { latestBrowserState { snapshotId reason groups { logicalGroupId title browserGroupId tabUrls } } }",
    );
    assert_ok("latestBrowserState", &latest);
    assert!(
        response_data(&latest)["latestBrowserState"]["groups"]
            .as_array()
            .is_some_and(|groups| groups.iter().any(|group| {
                group["browserGroupId"].as_i64() == Some(group_id)
                    && group["title"].as_str() == Some(group_name.as_str())
            })),
        "expected latest browser state to include test group: {latest}"
    );

    let history = b.run_query(
        "query { browserStateHistory(limit: 5) { snapshotId reason eventCount eventKinds } }",
    );
    assert_ok("browserStateHistory", &history);
    assert!(
        response_data(&history)["browserStateHistory"]
            .as_array()
            .is_some_and(|entries| !entries.is_empty()),
        "expected persisted browser-state history entries: {history}"
    );

    let group_history = b.run_query(&format!(
        "query {{ browserStateGroupHistory(title: {group_name_gql}, limit: 10) {{ logicalGroupId title browserGroupId tabUrls }} }}"
    ));
    assert_ok("browserStateGroupHistory", &group_history);
    assert!(
        response_data(&group_history)["browserStateGroupHistory"]
            .as_array()
            .is_some_and(|entries| entries.iter().any(|entry| {
                entry["browserGroupId"].as_i64() == Some(group_id)
                    && entry["title"].as_str() == Some(group_name.as_str())
            })),
        "expected persisted group history for test group: {group_history}"
    );

    b.close_test_window(window_id);
}

#[test]
#[ignore = "requires built dist artifacts and Chrome"]
fn test_tab_graphql_workflows() {
    let b = shared_browser();
    let (window_id, tab_ids) = b.create_test_window(
        &[
            "https://example.com",
            "https://example.org",
            "https://example.net",
        ],
        None,
    );
    sleep(Duration::from_secs(2));

    let focus = b.run_query(&format!(
        "mutation {{ focusTab(tabId: {}) {{ success tabId }} }}",
        tab_ids[0]
    ));
    assert_ok("focusTab", &focus);
    assert_eq!(
        response_data(&focus)["focusTab"]["success"].as_bool(),
        Some(true)
    );

    let refresh = b.run_query(&format!(
        "mutation {{ refreshTabs(tabIds: [{}]) {{ refreshedTabs }} }}",
        tab_ids[0]
    ));
    assert_ok("refreshTabs", &refresh);
    assert_eq!(
        response_data(&refresh)["refreshTabs"]["refreshedTabs"].as_i64(),
        Some(1)
    );

    let move_tab = b.run_query(&format!(
        "mutation {{ moveTab(tabIds: [{}], windowId: {window_id}, index: 0) {{ movedTabs }} }}",
        tab_ids[2]
    ));
    assert_ok("moveTab", &move_tab);
    assert_eq!(
        response_data(&move_tab)["moveTab"]["movedTabs"].as_i64(),
        Some(1)
    );
    sleep(Duration::from_secs(1));

    let reordered = window_query(b, window_id);
    let ordered_tab_ids: Vec<i64> = response_data(&reordered)["window"]["tabs"]
        .as_array()
        .expect("window tabs")
        .iter()
        .filter_map(|tab| tab["tabId"].as_i64())
        .collect();
    assert_eq!(ordered_tab_ids.first().copied(), Some(tab_ids[2]));

    let close = b.run_query(&format!(
        "mutation {{ closeTabs(tabIds: [{}], confirm: true) {{ txid closedTabs remainingTabs {{ tabId }} }} }}",
        tab_ids[1]
    ));
    assert_ok("closeTabs", &close);
    assert_eq!(
        response_data(&close)["closeTabs"]["closedTabs"].as_i64(),
        Some(1)
    );

    let undo = b.run_query("mutation { undoAction(latest: true) { txid summary } }");
    assert_ok("undoAction", &undo);
    sleep(Duration::from_secs(1));

    let restored = window_query(b, window_id);
    assert!(
        response_data(&restored)["window"]["tabs"]
            .as_array()
            .is_some_and(|tabs| {
                tabs.len() == 3
                    && tabs
                        .iter()
                        .any(|tab| tab["url"].as_str() == Some("https://example.org/"))
            }),
        "expected undoAction to restore the closed tab: {restored}"
    );

    b.close_test_window(window_id);
}

#[test]
#[ignore = "requires built dist artifacts and Chrome"]
fn test_archive_dedupe_and_history_graphql_workflows() {
    let b = shared_browser();
    let open = b.send_host_request(
        "open",
        json!({
            "newWindow": true,
            "urls": [
                "https://example.com/?dup=1",
                "https://example.com/?dup=1",
                "https://example.org/?dup=1"
            ],
            "allowDuplicates": true
        }),
    );
    let window_id = response_data(&open)["windowId"]
        .as_i64()
        .expect("host open should return windowId");
    sleep(Duration::from_secs(2));

    let analyze = b.run_query(&format!(
        "query {{ analyze(windowId: {window_id}) {{ totalTabs duplicateTabs staleTabs }} }}"
    ));
    assert_ok("analyze", &analyze);
    assert!(
        response_data(&analyze)["analyze"]["duplicateTabs"]
            .as_i64()
            .unwrap_or(0)
            >= 1,
        "expected duplicate tabs to be detected: {analyze}"
    );

    let dedupe_preview = b.run_query(&format!(
        "mutation {{ deduplicateTabs(windowId: {window_id}) {{ closedTabs candidateTabs {{ tabId url }} }} }}"
    ));
    assert_ok("deduplicateTabs preview", &dedupe_preview);
    assert!(
        response_data(&dedupe_preview)["deduplicateTabs"]["candidateTabs"]
            .as_array()
            .is_some_and(|tabs| !tabs.is_empty()),
        "expected dedupe preview candidates: {dedupe_preview}"
    );

    let dedupe = b.run_query(&format!(
        "mutation {{ deduplicateTabs(windowId: {window_id}, confirm: true) {{ txid closedTabs }} }}"
    ));
    assert_ok("deduplicateTabs confirm", &dedupe);
    assert!(
        response_data(&dedupe)["deduplicateTabs"]["closedTabs"]
            .as_i64()
            .unwrap_or(0)
            >= 1,
        "expected dedupe confirm to close duplicates: {dedupe}"
    );

    let archive = b.run_query(&format!(
        "mutation {{ archiveTabs(windowId: {window_id}) {{ txid archivedTabs }} }}"
    ));
    assert_ok("archiveTabs", &archive);
    let archive_txid = response_data(&archive)["archiveTabs"]["txid"]
        .as_str()
        .map(ToOwned::to_owned)
        .expect("archiveTabs should return txid");
    assert!(
        response_data(&archive)["archiveTabs"]["archivedTabs"]
            .as_i64()
            .unwrap_or(0)
            >= 1,
        "expected archiveTabs to archive at least one tab: {archive}"
    );

    sleep(Duration::from_secs(1));
    let history = b.run_query("query { history(limit: 5) { txid action summary } }");
    assert_ok("history", &history);
    assert!(
        response_data(&history)["history"]
            .as_array()
            .is_some_and(|entries| {
                entries
                    .iter()
                    .any(|entry| entry["txid"].as_str() == Some(archive_txid.as_str()))
            }),
        "expected history entries after archive: {history}"
    );

    let undo = b.run_query("mutation { undoAction(latest: true) { txid summary } }");
    assert_ok("undoAction latest", &undo);

    b.close_test_window(window_id);
}

#[test]
#[ignore = "requires built dist artifacts and Chrome"]
fn test_inspect_report_screenshot_and_reload_graphql_workflows() {
    let b = shared_browser();
    let (window_id, tab_ids) = b.create_test_window(&["https://example.com"], None);
    let tab_id = tab_ids[0];
    sleep(Duration::from_secs(2));

    let inspect = b.run_query(&format!(
        r#"query {{ inspectTabs(tabIds: [{tab_id}], signals: ["page-meta"]) {{ totals {{ tabs signals tasks }} entries {{ tabId signals {{ name valueJson }} }} }} }}"#
    ));
    assert_ok("inspectTabs", &inspect);
    assert_eq!(
        response_data(&inspect)["inspectTabs"]["totals"]["tabs"].as_i64(),
        Some(1)
    );

    let report = b.run_query(&format!(
        "query {{ reportTabs(windowId: {window_id}) {{ totals {{ tabs }} entries {{ tabId url description }} }} }}"
    ));
    assert_ok("reportTabs", &report);
    assert!(
        response_data(&report)["reportTabs"]["entries"]
            .as_array()
            .is_some_and(|entries| entries.iter().any(|entry| {
                entry["tabId"].as_i64() == Some(tab_id) && entry["description"].as_str().is_some()
            })),
        "expected report entry for example tab: {report}"
    );

    let screenshot = b.run_query(&format!(
        r#"query {{ captureScreenshots(tabIds: [{tab_id}], mode: "viewport") {{ totals {{ tabs tiles }} entries {{ tabId tiles {{ index width height }} error {{ message }} }} }} }}"#
    ));
    assert_ok("captureScreenshots", &screenshot);
    assert_eq!(
        response_data(&screenshot)["captureScreenshots"]["totals"]["tabs"].as_i64(),
        Some(1)
    );

    let reload = b.run_query("mutation { reloadExtension { reloading } }");
    assert_ok("reloadExtension", &reload);
    assert_eq!(
        response_data(&reload)["reloadExtension"]["reloading"].as_bool(),
        Some(true)
    );

    b.close_test_window(window_id);
}
