use super::common::*;
use serde_json::{json, Value};
use std::fs;
use std::thread::sleep;
use std::time::{Duration, Instant};

struct TestWindow<'a> {
    browser: &'a SharedBrowser,
    id: i64,
}

impl Drop for TestWindow<'_> {
    fn drop(&mut self) {
        self.browser.close_test_window(self.id);
    }
}

fn snapshot(browser: &SharedBrowser, window: i64) -> Value {
    let result = browser.run_query(&format!(
        "query {{ window(id: {window}) {{ tabs {{ tabId url index active pinned groupId groupTitle }} groups {{ groupId title color collapsed }} }} }}"
    ));
    response_data(&result)["window"].clone()
}

fn without_tab_ids(mut value: Value) -> Value {
    for tab in value["tabs"].as_array_mut().expect("snapshot tabs") {
        tab.as_object_mut().unwrap().remove("tabId");
    }
    value
}

fn txid(result: &Value, mutation: &str) -> String {
    response_data(result)[mutation]["txid"]
        .as_str()
        .filter(|value| !value.is_empty())
        .expect("mutation must return a durable transaction")
        .into()
}

#[test]
#[ignore = "requires built dist artifacts and Chrome or Edge"]
fn close_preview_and_empty_scope_never_mutate() {
    let b = shared_browser();
    let (id, tabs) =
        b.create_test_window(&["about:blank?preview-a", "about:blank?preview-b"], None);
    let _window = TestWindow { browser: b, id };
    let before = snapshot(b, id);
    for options in ["", ", confirm: false", ", confirm: true, dryRun: true"] {
        let result = b.run_query(&format!(
            "mutation {{ closeTabs(tabIds: [{}]{options}) {{ closedTabs }} }}",
            tabs[0]
        ));
        assert_eq!(response_data(&result)["closeTabs"]["closedTabs"], 0);
        assert_eq!(snapshot(b, id), before);
    }
    let _empty = b.output(&[
        "query",
        "mutation { closeTabs(tabIds: [], confirm: true) { closedTabs } }",
    ]);
    assert_eq!(snapshot(b, id), before, "empty selection broadened scope");
}

#[test]
#[ignore = "requires built dist artifacts and Chrome or Edge"]
fn close_undo_restores_order_surviving_group_and_active_tab_once() {
    let b = shared_browser();
    let (id, tabs) = b.create_test_window(
        &[
            "about:blank?first",
            "about:blank?middle",
            "about:blank?last",
        ],
        Some("TEST-Undo-Exact"),
    );
    let _window = TestWindow { browser: b, id };
    b.run_query(&format!(
        "mutation {{ focusTab(tabId: {}) {{ success }} }}",
        tabs[1]
    ));
    let before = without_tab_ids(snapshot(b, id));
    let close = b.run_query(&format!(
        "mutation {{ closeTabs(tabIds: [{}], confirm: true) {{ txid closedTabs undoUnavailable }} }}",
        tabs[1]
    ));
    assert_eq!(response_data(&close)["closeTabs"]["closedTabs"], 1);
    assert_eq!(
        response_data(&close)["closeTabs"].get("undoUnavailable"),
        Some(&Value::Null)
    );
    let transaction = txid(&close, "closeTabs");
    b.run_query(&format!(
        "mutation {{ undoAction(txid: {}) {{ txid summary }} }}",
        gql_string(&transaction)
    ));
    assert_eq!(without_tab_ids(snapshot(b, id)), before);
    let restored = snapshot(b, id);
    let replay = b.output(&[
        "query",
        &format!(
            "mutation {{ undoAction(txid: {}) {{ txid }} }}",
            gql_string(&transaction)
        ),
    ]);
    assert!(
        !replay.status.success(),
        "undo replay must be explicitly rejected"
    );
    assert_eq!(snapshot(b, id), restored, "undo replay duplicated tabs");
}

#[test]
#[ignore = "requires built dist artifacts and Chrome or Edge"]
fn open_exposes_exact_transaction_for_undo() {
    let b = shared_browser();
    let (id, _) = b.create_test_window(&["about:blank?open-keep"], None);
    let _window = TestWindow { browser: b, id };
    let before = snapshot(b, id);
    let opened = b.run_query(&format!(
        "mutation {{ openTabs(urls: [\"about:blank?open-one\", \"about:blank?open-two\"], windowId: {id}) {{ txid undoUnavailable tabs {{ tabId }} }} }}"
    ));
    assert_eq!(
        response_data(&opened)["openTabs"]["tabs"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        response_data(&opened)["openTabs"].get("undoUnavailable"),
        Some(&Value::Null)
    );
    let transaction = txid(&opened, "openTabs");
    let undo = b.run_query(&format!(
        "mutation {{ undoAction(txid: {}) {{ txid }} }}",
        gql_string(&transaction)
    ));
    assert_eq!(response_data(&undo)["undoAction"]["txid"], transaction);
    assert_eq!(snapshot(b, id), before);
}

#[test]
#[ignore = "requires built dist artifacts and Chrome or Edge"]
fn dedupe_is_durable_and_reversible() {
    let b = shared_browser();
    let page = HttpFixture::html("<title>Dedupe fixture</title><main>Duplicate</main>".into());
    let first = format!("{}/same", page.url);
    let last = format!("{}/last", page.url);
    let (id, tabs) = b.create_test_window(&[&first, &last], Some("TEST-Dedupe"));
    let _window = TestWindow { browser: b, id };
    let duplicate = b.seed_browser(
        "tab-create",
        json!({"windowId":id,"url":first,"active":false,"index":1}),
    );
    assert_ok("create duplicate", &duplicate);
    let duplicate_id = duplicate["id"].as_i64().expect("duplicate tab ID");
    b.run_query(&format!(
        "mutation {{ assignToGroup(tabIds: [{duplicate_id}], groupTitle: \"TEST-Dedupe\") {{ groupId }} }}"
    ));
    b.run_query(&format!(
        "mutation {{ focusTab(tabId: {}) {{ success }} }}",
        tabs[0]
    ));
    let before = without_tab_ids(snapshot(b, id));
    let result = b.run_query(&format!(
        "mutation {{ deduplicateTabs(windowId: {id}, confirm: true) {{ txid closedTabs }} }}"
    ));
    assert_eq!(
        response_data(&result)["deduplicateTabs"]["closedTabs"],
        1,
        "{result}"
    );
    let transaction = txid(&result, "deduplicateTabs");
    let history = b.run_query("query { history(limit: 20) { txid action } }");
    assert!(response_data(&history)["history"]
        .as_array()
        .unwrap()
        .iter()
        .any(|entry| entry["txid"] == transaction));
    b.run_query(&format!(
        "mutation {{ undoAction(txid: {}) {{ txid }} }}",
        gql_string(&transaction)
    ));
    assert_eq!(without_tab_ids(snapshot(b, id)), before);
}

struct PolicyGuard {
    path: std::path::PathBuf,
    previous: Option<Vec<u8>>,
}

impl Drop for PolicyGuard {
    fn drop(&mut self) {
        let result = match &self.previous {
            Some(previous) => fs::write(&self.path, previous),
            None => fs::remove_file(&self.path),
        };
        if let Err(error) = result {
            eprintln!("restore test policy: {error}");
        }
    }
}

#[test]
#[ignore = "requires built dist artifacts and Chrome or Edge"]
fn policy_protects_pinned_tabs_through_the_real_host() {
    let b = shared_browser();
    let (id, tabs) = b.create_test_window(&["about:blank?pinned", "about:blank?keep"], None);
    let _window = TestWindow { browser: b, id };
    let pinned = b.seed_browser("tab-update", json!({"tabId":tabs[0],"pinned":true}));
    assert_ok("pin test tab", &pinned);
    assert_eq!(pinned["pinned"], true, "real browser tab was not pinned");
    let path = b.config_home.join("tabctl/policy.json");
    let previous = match fs::read(&path) {
        Ok(previous) => Some(previous),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => panic!("read sandbox policy: {error}"),
    };
    let _policy = PolicyGuard {
        path: path.clone(),
        previous,
    };
    fs::write(&path, r#"{"protect":{"pinned":true}}"#).unwrap();
    let before = snapshot(b, id);
    let _result = b.output(&[
        "query",
        &format!(
            "mutation {{ closeTabs(tabIds: [{}], confirm: true) {{ closedTabs }} }}",
            tabs[0]
        ),
    ]);
    assert_eq!(
        snapshot(b, id),
        before,
        "policy allowed protected tab removal"
    );
    let duplicate = b.seed_browser(
        "tab-create",
        json!({"windowId":id,"url":"about:blank?pinned","pinned":true,"active":false}),
    );
    assert_ok("create protected duplicate", &duplicate);
    b.run_query(&format!(
        "mutation {{ assignToGroup(tabIds: [{}], groupTitle: \"TEST-Policy-Keep\") {{ groupId }} }}",
        tabs[1]
    ));
    fs::write(
        &path,
        r#"{"protect":{"pinned":true,"groupTitles":["TEST-Policy"]}}"#,
    )
    .unwrap();
    let before = snapshot(b, id);
    for (field, mutation) in [
        ("archiveTabs", format!("archiveTabs(windowId: {id}) {{ txid undoUnavailable archivedTabs skippedTabs skipped {{ tabId reason }} }}")),
        ("deduplicateTabs", format!("deduplicateTabs(windowId: {id}, confirm: true) {{ txid undoUnavailable closedTabs skippedTabs skipped {{ tabId reason }} }}")),
        ("closeTabs", format!(
            "closeTabs(tabIds: [{}], confirm: true) {{ closedTabs }}",
            tabs[1]
        )),
    ] {
        let result = b.run_query(&format!("mutation {{ {mutation} }}"));
        assert_eq!(snapshot(b, id), before, "policy was bypassed by {mutation}");
        if field != "closeTabs" {
            let outcome = &response_data(&result)[field];
            assert_eq!(outcome.get("txid"), Some(&Value::Null));
            let skipped = outcome["skipped"].as_array().expect("policy exclusions");
            assert!(!skipped.is_empty(), "policy reasons lost: {outcome}");
            assert_eq!(outcome["skippedTabs"].as_u64(), Some(skipped.len() as u64));
            assert!(skipped.iter().all(|tab| tab["reason"]
                .as_str()
                .unwrap()
                .starts_with("protected_")));
        }
    }
}

#[test]
#[ignore = "requires built dist artifacts and Chrome or Edge"]
fn partial_group_failure_records_recovery_and_cli_reports_errors() {
    let b = shared_browser();
    let (id, tabs) = b.create_test_window(&["about:blank?partial", "about:blank?keep"], None);
    let _window = TestWindow { browser: b, id };
    let before = snapshot(b, id);
    let prior_history = b.run_query("query { history(limit: 100) { txid } }");
    let prior_ids: Vec<_> = response_data(&prior_history)["history"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["txid"].clone())
        .collect();
    let failed = b.output(&[
        "query",
        &format!("mutation {{ assignToGroup(tabIds: [{}], groupTitle: \"TEST-Partial\", color: \"invalid-color\") {{ groupId }} }}", tabs[0]),
    ]);
    assert!(!failed.status.success(), "GraphQL errors must fail the CLI");
    let response: Value = serde_json::from_slice(&failed.stdout).expect("error JSON is retained");
    assert!(!response["errors"]
        .as_array()
        .expect("GraphQL errors")
        .is_empty());
    let changed = snapshot(b, id);
    assert_ne!(
        changed, before,
        "fixture must exercise a real partially completed mutation"
    );
    if changed != before {
        let history = b.run_query("query { history(limit: 100) { txid } }");
        let recovery = response_data(&history)["history"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| !prior_ids.contains(&entry["txid"]))
            .expect("partially completed mutation lost its recovery transaction");
        b.run_query(&format!(
            "mutation {{ undoAction(txid: {}) {{ txid }} }}",
            recovery["txid"]
        ));
        assert_eq!(
            snapshot(b, id),
            before,
            "partial mutation undo did not restore ungrouped tabs"
        );
    }
    let failed = b.output(&[
        "query",
        "mutation { focusTab(tabId: 2147483647) { success } }",
    ]);
    assert!(!failed.status.success());
    let error: Value = serde_json::from_slice(&failed.stdout).expect("structured GraphQL error");
    assert!(!error["errors"].as_array().unwrap().is_empty());
}

#[test]
#[ignore = "requires built dist artifacts and Chrome or Edge"]
fn large_escaped_unicode_pages_do_not_disconnect_native_messaging() {
    let b = shared_browser();
    let max_native_bytes = 10 * 1024 * 1024;
    let html = format!(
        "<!doctype html><html><head><meta charset=utf-8><title>Large payload fixture</title></head><body><main><h1>Bounded payload</h1><p>Visible content remains readable while the document carries a large hidden payload.</p><pre hidden>{}</pre></main></body></html>",
        "\"\\\u{1f600}".repeat(1_600_000)
    );
    assert!(serde_json::to_vec(&html).unwrap().len() > max_native_bytes);
    assert!(html.encode_utf16().count() < max_native_bytes);
    let page = HttpFixture::html(html);
    let (id, tabs) = b.create_test_window(&[&page.url], None);
    let _window = TestWindow { browser: b, id };
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let result = b.run_query(&format!(
            "query {{ tab(id: {}) {{ title url status }} }}",
            tabs[0]
        ));
        let tab = &response_data(&result)["tab"];
        if tab["title"] == "Large payload fixture" && tab["status"] == "complete" {
            assert_eq!(tab["url"], format!("{}/", page.url));
            assert!(page.completed_pages() > 0, "{}", page.diagnostics());
            break;
        }
        assert!(
            Instant::now() < deadline,
            "fixture page never loaded; GraphQL={result}; HTTP={}",
            page.diagnostics()
        );
        sleep(Duration::from_millis(100));
    }
    for _ in 0..2 {
        let read = b.output(&[
            "query",
            &format!(
                "query {{ readTabs(tabIds: [{}], extract: false, maxHtmlChars: {max_native_bytes}, maxChars: 4096) {{ entries {{ tabId markdown error cached status diagnostics {{ source documentReadyState truncatedHtml sourceHtmlChars }} }} }} }}",
                tabs[0]
            ),
        ]);
        let result: Value = serde_json::from_slice(&read.stdout).expect("large read returned JSON");
        assert!(read.status.success(), "large-page read failed: {result}");
        let entry = &response_data(&result)["readTabs"]["entries"][0];
        assert_eq!(entry["tabId"], tabs[0]);
        assert!(
            entry["error"].is_null(),
            "large-page extraction failed: {entry}"
        );
        assert_eq!(entry["status"], "READ");
        assert_eq!(entry["cached"], false);
        assert_eq!(entry["diagnostics"]["source"], "live");
        assert_eq!(entry["diagnostics"]["documentReadyState"], "complete");
        assert!(entry["markdown"]
            .as_str()
            .unwrap()
            .contains("Bounded payload"));
        assert_eq!(
            entry["diagnostics"]["truncatedHtml"], true,
            "UTF-8/envelope truncation was hidden: {entry}"
        );
        let source_chars = entry["diagnostics"]["sourceHtmlChars"].as_u64().unwrap();
        assert!(
            (6_400_000..max_native_bytes as u64).contains(&source_chars),
            "HTML must exceed the encoded byte budget, not the requested character cap"
        );
        let ping = b.run(&["ping"]);
        let ping = response_data(&ping);
        assert_eq!(ping["runtimeId"], b.extension_id);
        assert_eq!(ping["nativeChannelAvailable"], true);
        for field in ["version", "hostVersion"] {
            assert!(ping[field]
                .as_str()
                .is_some_and(|version| !version.is_empty()));
        }
    }
}
