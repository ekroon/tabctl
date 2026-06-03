//! Browser-backed integration smoke test.
//!
//! Uses the shared Chrome instance from `common::shared_browser()` so the
//! bootstrap cost is paid only once across all integration test files.

mod common;

use common::*;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::thread::sleep;
use std::time::Duration;

fn known_markdown_fixture_url() -> String {
    let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind known markdown fixture");
    let addr = listener.local_addr().expect("read fixture address");
    std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut request_buffer = [0_u8; 1024];
            let _ = stream.read(&mut request_buffer);
            let body = r#"<!doctype html>
<html lang="en">
  <head>
    <meta charset="utf-8">
    <title>Known Markdown Fixture</title>
  </head>
  <body>
    <header>
      <h1>Navigation Shell Heading</h1>
      <p>This noisy shell text should not dominate readTabs Markdown output.</p>
    </header>
    <aside>
      <h2>Debug Panel</h2>
      <p>Response timing, staff tools, and route diagnostics appear before content.</p>
    </aside>
    <main>
      <h1>Known Markdown Heading</h1>
      <p>This deterministic integration paragraph proves readTabs converted fixture HTML.</p>
      <ul>
        <li>First known list item</li>
        <li>Second known list item</li>
      </ul>
      <a href="https://example.test/fixture-link">Fixture Link</a>
    </main>
  </body>
</html>"#;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    format!("http://{addr}/known-markdown")
}

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

#[test]
#[ignore = "requires built dist artifacts and Chrome"]
fn read_tabs_returns_markdown_for_known_page() {
    let b = shared_browser();
    let fixture_url = known_markdown_fixture_url();
    let (window_id, tab_ids) = b.create_test_window(&[fixture_url.as_str()], None);
    let tab_id = tab_ids[0];
    sleep(Duration::from_secs(1));

    let read = b.run_query(&format!(
        r#"query {{ readTabs(tabIds: [{tab_id}], extract: true, maxChars: 50000, timeoutMs: 15000) {{ totals {{ tabs tasks }} entries {{ tabId url title chars truncated extracted cached status emptyReason diagnostics {{ source cachedAt cacheAgeMs sourceHtmlChars sourceTextChars documentReadyState truncatedHtml }} error markdown }} }} }}"#
    ));
    assert_ok("readTabs known markdown page", &read);
    let data = &response_data(&read)["readTabs"];
    assert_eq!(
        data["totals"]["tabs"].as_i64(),
        Some(1),
        "readTabs should return one tab: {read}"
    );
    assert_eq!(
        data["totals"]["tasks"].as_i64(),
        Some(1),
        "readTabs should run extraction for the known http page: {read}"
    );

    let entry = &data["entries"][0];
    assert_eq!(entry["tabId"].as_i64(), Some(tab_id));
    assert_eq!(entry["status"].as_str(), Some("READ"), "{read}");
    assert_eq!(entry["emptyReason"], serde_json::Value::Null);
    assert_eq!(entry["error"], serde_json::Value::Null);
    assert_eq!(entry["extracted"].as_bool(), Some(true));
    assert_eq!(entry["cached"].as_bool(), Some(false));
    assert_eq!(entry["truncated"].as_bool(), Some(false));
    assert!(
        entry["chars"].as_i64().unwrap_or(0) > 0,
        "readTabs should report Markdown characters: {read}"
    );
    assert!(
        entry["diagnostics"]["sourceHtmlChars"]
            .as_i64()
            .unwrap_or(0)
            > 0,
        "readTabs should report source HTML diagnostics: {read}"
    );
    assert!(
        entry["diagnostics"]["sourceTextChars"]
            .as_i64()
            .unwrap_or(0)
            > 0,
        "readTabs should report source text diagnostics: {read}"
    );
    assert_eq!(entry["diagnostics"]["source"].as_str(), Some("live"));
    assert_eq!(entry["diagnostics"]["cachedAt"], serde_json::Value::Null);
    assert_eq!(entry["diagnostics"]["cacheAgeMs"], serde_json::Value::Null);
    assert_eq!(
        entry["diagnostics"]["documentReadyState"].as_str(),
        Some("complete")
    );
    assert_eq!(entry["diagnostics"]["truncatedHtml"].as_bool(), Some(false));

    let markdown = entry["markdown"]
        .as_str()
        .expect("readTabs should return Markdown text");
    assert!(
        markdown.contains("Known Markdown Heading"),
        "expected known heading in Markdown: {markdown}"
    );
    assert!(
        markdown.contains("deterministic integration paragraph"),
        "expected known paragraph in Markdown: {markdown}"
    );
    assert!(
        markdown.contains("First known list item"),
        "expected known list item in Markdown: {markdown}"
    );
    assert!(
        markdown.contains("Fixture Link") && markdown.contains("https://example.test/fixture-link"),
        "expected known link in Markdown: {markdown}"
    );
    assert!(
        !markdown.contains("Navigation Shell Heading") && !markdown.contains("Debug Panel"),
        "expected readTabs to prefer article content over page chrome: {markdown}"
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
