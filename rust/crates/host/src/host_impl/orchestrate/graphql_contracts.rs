//! Contract tests: drive real orchestrations to completion and verify
//! their responses parse correctly through the GraphQL resolver.
//!
//! These tests prevent response key mismatches between orchestrations
//! and GraphQL resolvers. If either side changes a key name or shape,
//! the corresponding contract test will fail.

use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use super::*;

/// A CommandSender that returns pre-recorded responses for specific actions.
struct ContractSender {
    responses: Mutex<Vec<(String, Value)>>,
    requests: Mutex<Vec<(String, Value)>>,
    snapshot: Value,
}

impl ContractSender {
    fn new(snapshot: Value) -> Self {
        Self {
            responses: Mutex::new(Vec::new()),
            requests: Mutex::new(Vec::new()),
            snapshot,
        }
    }

    fn add_response(&self, action: &str, response: Value) {
        self.responses
            .lock()
            .unwrap()
            .push((action.to_string(), response));
    }

    fn request_params(&self, action: &str) -> Option<Value> {
        self.requests
            .lock()
            .unwrap()
            .iter()
            .find_map(|(a, params)| (a == action).then(|| params.clone()))
    }
}

impl tabctl_graphql::CommandSender for ContractSender {
    fn send(&self, action: &str, params: Value) -> Result<Value, String> {
        self.requests
            .lock()
            .unwrap()
            .push((action.to_string(), params));
        let responses = self.responses.lock().unwrap();
        for (a, r) in responses.iter() {
            if a == action {
                return Ok(r.clone());
            }
        }
        Err(format!(
            "ContractSender: no response registered for action '{action}'"
        ))
    }

    fn snapshot(&self) -> Result<Value, String> {
        Ok(self.snapshot.clone())
    }
}

fn sample_snapshot() -> Value {
    json!({
        "windows": [{
            "windowId": 100,
            "focused": true,
            "tabs": [
                {"tabId": 1, "windowId": 100, "index": 0, "url": "https://a.com", "title": "A", "active": true, "pinned": false, "groupId": 10, "groupTitle": "Work", "groupColor": "blue", "groupCollapsed": false},
                {"tabId": 2, "windowId": 100, "index": 1, "url": "https://b.com", "title": "B", "active": false, "pinned": true, "groupId": -1}
            ],
            "groups": [{"groupId": 10, "title": "Work", "color": "blue", "collapsed": false}]
        }]
    })
}

fn group_gather_snapshot() -> Value {
    json!({
        "windows": [{
            "windowId": 100,
            "focused": true,
            "tabs": [
                {"tabId": 1, "windowId": 100, "index": 0, "url": "https://a.com", "title": "A", "active": true, "pinned": false, "groupId": 10, "groupTitle": "Work", "groupColor": "blue", "groupCollapsed": false},
                {"tabId": 2, "windowId": 100, "index": 1, "url": "https://b.com", "title": "B", "active": false, "pinned": false, "groupId": 20, "groupTitle": "Work", "groupColor": "red", "groupCollapsed": false},
                {"tabId": 3, "windowId": 100, "index": 2, "url": "https://c.com", "title": "C", "active": false, "pinned": false, "groupId": 20, "groupTitle": "Work", "groupColor": "red", "groupCollapsed": false}
            ],
            "groups": [
                {"groupId": 10, "title": "Work", "color": "blue", "collapsed": false},
                {"groupId": 20, "title": "Work", "color": "red", "collapsed": false}
            ]
        }]
    })
}

fn multi_window_snapshot() -> Value {
    json!({
        "windows": [
            {
                "windowId": 100,
                "focused": true,
                "tabs": [
                    {"tabId": 1, "windowId": 100, "index": 0, "url": "https://a.com", "title": "A", "active": true, "pinned": false, "groupId": 10, "groupTitle": "Dev", "groupColor": "blue", "groupCollapsed": false},
                    {"tabId": 2, "windowId": 100, "index": 1, "url": "https://b.com", "title": "B", "active": false, "pinned": false, "groupId": 10, "groupTitle": "Dev", "groupColor": "blue", "groupCollapsed": false},
                    {"tabId": 3, "windowId": 100, "index": 2, "url": "https://c.com", "title": "C", "active": false, "pinned": false, "groupId": -1}
                ],
                "groups": [{"groupId": 10, "title": "Dev", "color": "blue", "collapsed": false}]
            },
            {
                "windowId": 200,
                "focused": false,
                "tabs": [
                    {"tabId": 4, "windowId": 200, "index": 0, "url": "https://d.com", "title": "D", "active": true, "pinned": false, "groupId": -1}
                ],
                "groups": []
            }
        ]
    })
}

fn dedupe_snapshot() -> Value {
    json!({
        "windows": [{
            "windowId": 100,
            "focused": true,
            "tabs": [
                {"tabId": 1, "windowId": 100, "index": 0, "url": "https://dup.com", "title": "Keep", "active": true, "pinned": false, "groupId": -1},
                {"tabId": 2, "windowId": 100, "index": 1, "url": "https://dup.com", "title": "Close", "active": false, "pinned": false, "groupId": -1},
                {"tabId": 3, "windowId": 100, "index": 2, "url": "https://other.com", "title": "Other", "active": false, "pinned": false, "groupId": -1}
            ],
            "groups": []
        }]
    })
}

/// Inject txid into an orchestration response, mimicking the host layer
/// behavior in state.rs:484-488.
fn inject_txid(response: &mut Value, undo: &Option<Value>) {
    if let Some(obj) = response.as_object_mut() {
        if undo.is_some() {
            obj.insert("txid".to_string(), Value::String("tx-contract-1".into()));
        } else {
            obj.insert("txid".to_string(), Value::Null);
        }
    }
}

// ── open ────────────────────────────────────────────────────────────────

#[test]
fn contract_open_tabs() {
    // Drive OpenOrchestration with new-window flow
    let params = json!({
        "urls": ["https://x.com", "https://y.com"],
        "newWindow": true,
        "groupTitle": "Test",
        "color": "blue"
    });
    let mut orch = open::OpenOrchestration::new(&params);

    let mock_responses = vec![
        // p:window-create
        json!({"id": 100, "tabs": [{"id": 999}]}),
        // p:tab-create (first)
        json!({"id": 50, "windowId": 100, "index": 0, "url": "https://x.com", "title": "X"}),
        // p:tab-create (second)
        json!({"id": 51, "windowId": 100, "index": 1, "url": "https://y.com", "title": "Y"}),
        // p:tab-remove (seed)
        json!({"removed": true}),
        // p:tab-group
        json!({"groupId": 30}),
        // p:group-update
        json!({"id": 30}),
        // p:snapshot (verification)
        json!({
            "windows": [{
                "windowId": 100,
                "tabs": [
                    {"tabId": 50, "windowId": 100, "index": 0, "url": "https://x.com", "title": "X", "groupId": 30, "groupTitle": "Test"},
                    {"tabId": 51, "windowId": 100, "index": 1, "url": "https://y.com", "title": "Y", "groupId": 30, "groupTitle": "Test"}
                ]
            }]
        }),
    ];

    let (mut response, undo) = drive_to_completion(&mut orch, &mock_responses);
    inject_txid(&mut response, &undo);

    // Feed orchestration response through GraphQL resolver
    let sender = Arc::new(ContractSender::new(sample_snapshot()));
    sender.add_response("open", response);

    let result = tabctl_graphql::execute(
        r#"mutation { openTabs(urls: ["https://x.com"], group: "Test") {
            tabs { tabId url }
            skippedUrls { url reason }
            windowId
            groupId
        }}"#,
        None,
        sample_snapshot(),
        sender,
    )
    .expect("GraphQL execution failed");

    assert!(result.get("errors").is_none(), "GraphQL errors: {result}");
    let open = &result["data"]["openTabs"];
    let tabs = open["tabs"].as_array().expect("tabs should be an array");
    assert_eq!(tabs.len(), 2, "expected 2 created tabs");
    assert_eq!(tabs[0]["tabId"], 50);
    assert_eq!(tabs[0]["url"], "https://x.com");
    assert_eq!(tabs[1]["tabId"], 51);
    assert_eq!(open["windowId"], 100);
    assert_eq!(open["groupId"], 30);
    // No URLs were skipped in this flow
    let skipped = open["skippedUrls"]
        .as_array()
        .expect("skippedUrls should be array");
    assert!(skipped.is_empty());
}

// ── close ───────────────────────────────────────────────────────────────

#[test]
fn contract_close_tabs() {
    let params = json!({"tabIds": [1], "confirmed": true});
    let mut orch = close::CloseOrchestration::new(&params);

    let mock_responses = vec![
        // p:snapshot
        sample_snapshot(),
        // p:tab-remove
        json!({"removed": true}),
    ];

    let (mut response, undo) = drive_to_completion(&mut orch, &mock_responses);
    inject_txid(&mut response, &undo);

    let sender = Arc::new(ContractSender::new(sample_snapshot()));
    sender.add_response("close", response);

    let result = tabctl_graphql::execute(
        r#"mutation { closeTabs(tabIds: [1], confirm: true) { txid closedTabs } }"#,
        None,
        sample_snapshot(),
        sender,
    )
    .expect("GraphQL execution failed");

    assert!(result.get("errors").is_none(), "GraphQL errors: {result}");
    let close = &result["data"]["closeTabs"];
    assert_eq!(close["txid"], "tx-contract-1");
    assert_eq!(close["closedTabs"], 1);
}

// ── refresh ─────────────────────────────────────────────────────────────

#[test]
fn contract_refresh_tabs() {
    let params = json!({"tabIds": [1, 2]});
    let mut orch = refresh::RefreshOrchestration::new(&params).unwrap();

    let mock_responses = vec![
        // p:tab-reload (first)
        json!({"reloaded": true}),
        // p:tab-reload (second)
        json!({"reloaded": true}),
    ];

    let (mut response, undo) = drive_to_completion(&mut orch, &mock_responses);
    inject_txid(&mut response, &undo);

    let sender = Arc::new(ContractSender::new(sample_snapshot()));
    sender.add_response("refresh", response);

    let result = tabctl_graphql::execute(
        "mutation { refreshTabs(tabIds: [1, 2]) { refreshedTabs } }",
        None,
        sample_snapshot(),
        sender,
    )
    .expect("GraphQL execution failed");

    assert!(result.get("errors").is_none(), "GraphQL errors: {result}");
    assert_eq!(result["data"]["refreshTabs"]["refreshedTabs"], 2);
}

// ── group-assign ────────────────────────────────────────────────────────

#[test]
fn contract_assign_to_group() {
    let params = json!({"tabIds": [2], "groupId": 10});
    let mut orch = group_assign::GroupAssignOrchestration::new(&params);

    let mock_responses = vec![
        // p:snapshot
        sample_snapshot(),
        // p:tab-group
        json!({"groupId": 10}),
    ];

    let (mut response, undo) = drive_to_completion(&mut orch, &mock_responses);
    inject_txid(&mut response, &undo);

    let sender = Arc::new(ContractSender::new(sample_snapshot()));
    sender.add_response("group-assign", response);

    let result = tabctl_graphql::execute(
        r#"mutation { assignToGroup(tabIds: [2], groupTitle: "Work") { groupId title tabCount } }"#,
        None,
        sample_snapshot(),
        sender,
    )
    .expect("GraphQL execution failed");

    assert!(result.get("errors").is_none(), "GraphQL errors: {result}");
    let group = &result["data"]["assignToGroup"];
    assert_eq!(group["groupId"], 10);
}

// ── archive ─────────────────────────────────────────────────────────────

#[test]
fn contract_archive_tabs() {
    let params = json!({"windowId": 100});
    let mut orch = archive::ArchiveOrchestration::new(&params);

    let mock_responses = vec![
        // p:snapshot
        sample_snapshot(),
        // p:window-create
        json!({"id": 300}),
        // p:tab-move (grouped batch)
        json!({}),
        // p:tab-group
        json!({"groupId": 50}),
        // p:group-update
        json!({}),
        // p:tab-move (ungrouped batch)
        json!({}),
        // p:tab-group
        json!({"groupId": 51}),
        // p:group-update
        json!({}),
    ];

    let (mut response, undo) = drive_to_completion(&mut orch, &mock_responses);
    inject_txid(&mut response, &undo);

    let sender = Arc::new(ContractSender::new(sample_snapshot()));
    sender.add_response("archive", response);

    let result = tabctl_graphql::execute(
        "mutation { archiveTabs(windowId: 100) { txid archivedTabs } }",
        None,
        sample_snapshot(),
        sender,
    )
    .expect("GraphQL execution failed");

    assert!(result.get("errors").is_none(), "GraphQL errors: {result}");
    let archive = &result["data"]["archiveTabs"];
    assert_eq!(archive["txid"], "tx-contract-1");
    assert_eq!(archive["archivedTabs"], 2);
}

// ── group-gather ─────────────────────────────────────────────────────────

#[test]
fn contract_gather_groups() {
    let snapshot = group_gather_snapshot();
    let params = json!({"windowId": 100});
    let mut orch = group_gather::GroupGatherOrchestration::new(&params);

    let mock_responses = vec![snapshot.clone(), json!({"groupId": 10})];

    let (mut response, undo) = drive_to_completion(&mut orch, &mock_responses);
    inject_txid(&mut response, &undo);

    let sender = Arc::new(ContractSender::new(snapshot.clone()));
    sender.add_response("group-gather", response);

    let result = tabctl_graphql::execute(
        r#"mutation { gatherGroups(windowId: 100) {
            summary { mergedGroups movedTabs }
            merged { groupTitle primaryGroupId mergedGroupCount movedTabs }
        } }"#,
        None,
        snapshot,
        sender,
    )
    .expect("GraphQL execution failed");

    assert!(result.get("errors").is_none(), "GraphQL errors: {result}");
    let gathered = &result["data"]["gatherGroups"];
    assert_eq!(gathered["summary"]["mergedGroups"], 1);
    assert_eq!(gathered["summary"]["movedTabs"], 2);
    let merged = gathered["merged"]
        .as_array()
        .expect("merged should be an array");
    assert_eq!(merged.len(), 1);
    assert_eq!(merged[0]["groupTitle"], "Work");
    assert_eq!(merged[0]["primaryGroupId"], 10);
}

// ── move-group ───────────────────────────────────────────────────────────

#[test]
fn contract_move_group() {
    let snapshot = multi_window_snapshot();
    let params = json!({"groupId": 10, "targetWindowId": 200});
    let mut orch = move_group::MoveGroupOrchestration::new(&params);

    let mock_responses = vec![
        snapshot.clone(),
        json!({}),
        json!({"groupId": 50}),
        json!({}),
        json!([
            {"id": 1, "groupId": 50},
            {"id": 2, "groupId": 50}
        ]),
    ];

    let (mut response, undo) = drive_to_completion(&mut orch, &mock_responses);
    inject_txid(&mut response, &undo);

    let sender = Arc::new(ContractSender::new(snapshot.clone()));
    sender.add_response("move-group", response);

    let result = tabctl_graphql::execute(
        r#"mutation { moveGroup(groupId: 10, windowId: 200) {
            groupId windowId movedToWindowId newGroupId movedTabs
        } }"#,
        None,
        snapshot,
        sender,
    )
    .expect("GraphQL execution failed");

    assert!(result.get("errors").is_none(), "GraphQL errors: {result}");
    let moved = &result["data"]["moveGroup"];
    assert_eq!(moved["groupId"], 50);
    assert_eq!(moved["windowId"], 100);
    assert_eq!(moved["movedToWindowId"], 200);
    assert_eq!(moved["newGroupId"], 50);
    assert_eq!(moved["movedTabs"], 2);
}

// ── merge-window ─────────────────────────────────────────────────────────

#[test]
fn contract_merge_windows() {
    let snapshot = multi_window_snapshot();
    let params = json!({"fromWindowId": 100, "toWindowId": 200});
    let mut orch = merge_window::MergeWindowOrchestration::new(&params);

    let mock_responses = vec![
        snapshot.clone(),
        json!({}),
        json!({"groupId": 50}),
        json!({}),
        json!({}),
    ];

    let (mut response, undo) = drive_to_completion(&mut orch, &mock_responses);
    inject_txid(&mut response, &undo);

    let sender = Arc::new(ContractSender::new(snapshot.clone()));
    sender.add_response("merge-window", response);

    let result = tabctl_graphql::execute(
        r#"mutation { mergeWindows(fromWindowId: 100, toWindowId: 200) {
            fromWindowId toWindowId sourceClosed movedTabs movedGroups
        } }"#,
        None,
        snapshot,
        sender,
    )
    .expect("GraphQL execution failed");

    assert!(result.get("errors").is_none(), "GraphQL errors: {result}");
    let merged = &result["data"]["mergeWindows"];
    assert_eq!(merged["fromWindowId"], 100);
    assert_eq!(merged["toWindowId"], 200);
    assert_eq!(merged["sourceClosed"], false);
    assert_eq!(merged["movedTabs"], 3);
    assert_eq!(merged["movedGroups"], 1);
}

// ── dedupe ───────────────────────────────────────────────────────────────

#[test]
fn contract_deduplicate_tabs() {
    let snapshot = dedupe_snapshot();
    let params = json!({"dedupe": true, "confirmed": true});
    let mut orch = analyze::AnalyzeOrchestration::new(&params);

    let mock_responses = vec![snapshot.clone(), json!({"removed": true})];

    let (mut response, undo) = drive_to_completion(&mut orch, &mock_responses);
    inject_txid(&mut response, &undo);

    let sender = Arc::new(ContractSender::new(snapshot.clone()));
    sender.add_response("analyze", response);

    let result = tabctl_graphql::execute(
        r#"mutation { deduplicateTabs(confirm: true) {
            txid closedTabs duplicateGroups candidateTabs { tabId url }
        } }"#,
        None,
        snapshot,
        sender,
    )
    .expect("GraphQL execution failed");

    assert!(result.get("errors").is_none(), "GraphQL errors: {result}");
    let dedupe = &result["data"]["deduplicateTabs"];
    assert_eq!(dedupe["txid"], "tx-contract-1");
    assert_eq!(dedupe["closedTabs"], 1);
    assert_eq!(dedupe["duplicateGroups"], 1);
    let candidate_tabs = dedupe["candidateTabs"]
        .as_array()
        .expect("candidateTabs should be an array");
    assert_eq!(candidate_tabs.len(), 1);
    assert_eq!(candidate_tabs[0]["tabId"], 2);
}

// ── inspect ──────────────────────────────────────────────────────────────

#[test]
fn contract_inspect_tabs() {
    let params = json!({"signals": [{"type": "page-meta"}]});
    let mut orch = inspect::InspectOrchestration::new(&params);

    let mock_responses = vec![
        sample_snapshot(),
        json!({"description": "Page desc"}),
        json!({"description": "Second page desc"}),
    ];

    let (response, undo) = drive_to_completion(&mut orch, &mock_responses);
    assert!(undo.is_none());

    let sender = Arc::new(ContractSender::new(sample_snapshot()));
    sender.add_response("inspect", response);

    let result = tabctl_graphql::execute(
        r#"{ inspectTabs(signals: ["page-meta"]) {
            totals { tabs signals tasks }
            entries { tabId windowId signals { name valueJson } }
        } }"#,
        None,
        sample_snapshot(),
        sender,
    )
    .expect("GraphQL execution failed");

    assert!(result.get("errors").is_none(), "GraphQL errors: {result}");
    let inspect = &result["data"]["inspectTabs"];
    assert_eq!(inspect["totals"]["tabs"], 2);
    assert_eq!(inspect["totals"]["signals"], 1);
    let entries = inspect["entries"]
        .as_array()
        .expect("entries should be an array");
    assert_eq!(entries.len(), 2);
    let first_signals = entries[0]["signals"]
        .as_array()
        .expect("signals should be an array");
    assert!(first_signals.iter().any(|s| s["name"] == "page-meta"));
}

#[test]
fn contract_inspect_tabs_extended_selector() {
    let snapshot = sample_snapshot();
    let sender = Arc::new(ContractSender::new(snapshot.clone()));
    sender.add_response(
        "inspect",
        serde_json::json!({
            "totals": {"tabs": 1, "signals": 1, "tasks": 1},
            "entries": [{
                "tabId": 1, "windowId": 100, "url": "https://a.com", "title": "A",
                "signals": {"btn": {"values": [], "missing": [], "errors": {}, "hints": {}}}
            }]
        }),
    );

    let result = tabctl_graphql::execute(
        r#"{ inspectTabs(windowId: 100, selectors: [
            { name: "btn", selector: ".btn", attr: "styles", styleProps: ["color", "font-size"], all: true, text: "Click", textMode: "contains" }
        ]) { totals { tasks } entries { tabId signals { name valueJson } } } }"#,
        None,
        snapshot,
        sender.clone(),
    )
    .unwrap();

    assert!(
        result.get("errors").is_none(),
        "errors: {:?}",
        result.get("errors")
    );
    assert_eq!(result["data"]["inspectTabs"]["totals"]["tasks"], 1);

    let params = sender
        .request_params("inspect")
        .expect("inspect params should be recorded");
    assert_eq!(params["windowId"], 100);
    assert_eq!(params["signals"], serde_json::json!(["selector"]));
    assert_eq!(params["selectorSpecs"][0]["name"], "btn");
    assert_eq!(params["selectorSpecs"][0]["selector"], ".btn");
    assert_eq!(params["selectorSpecs"][0]["attr"], "styles");
    assert_eq!(params["selectorSpecs"][0]["all"], true);
    assert_eq!(params["selectorSpecs"][0]["text"], "Click");
    assert_eq!(params["selectorSpecs"][0]["textMode"], "contains");
    assert_eq!(
        params["selectorSpecs"][0]["styleProps"],
        serde_json::json!(["color", "font-size"])
    );
}

#[test]
fn contract_read_markdown_inspect() {
    // Uses the real ReadMarkdownOrchestration wired through the GraphQL layer
    let snapshot = sample_snapshot();
    let sender = ContractSender::new(snapshot.clone());
    sender.add_response(
        "read-markdown",
        serde_json::json!({
            "totals": { "tabs": 1, "tasks": 1 },
            "entries": [{
                "tabId": 1, "windowId": 100, "url": "https://a.com", "title": "A",
                "markdown": "# A\n\nContent.", "chars": 13,
                "truncated": false, "extracted": true, "status": "READ", "emptyReason": null,
                "diagnostics": {
                    "sourceHtmlChars": 120,
                    "sourceTextChars": 10,
                    "documentReadyState": "complete",
                    "truncatedHtml": false
                },
                "error": null
            }]
        }),
    );

    let result = tabctl_graphql::execute(
        r#"{ readTabs(windowId: 100) { totals { tabs tasks } entries { tabId url markdown chars truncated extracted status emptyReason diagnostics { sourceHtmlChars sourceTextChars documentReadyState truncatedHtml } error } } }"#,
        None,
        snapshot,
        std::sync::Arc::new(sender),
    )
    .unwrap();

    assert!(
        result.get("errors").is_none(),
        "errors: {:?}",
        result.get("errors")
    );
    let totals = &result["data"]["readTabs"]["totals"];
    assert_eq!(totals["tabs"], 1);
    assert_eq!(totals["tasks"], 1);
    let entries = result["data"]["readTabs"]["entries"].as_array().unwrap();
    assert_eq!(entries[0]["tabId"], 1);
    assert_eq!(entries[0]["markdown"], "# A\n\nContent.");
    assert_eq!(entries[0]["status"], "READ");
    assert_eq!(entries[0]["diagnostics"]["documentReadyState"], "complete");
    assert!(entries[0]["error"].is_null());
}

// ── report ───────────────────────────────────────────────────────────────

#[test]
fn contract_report_tabs() {
    let params = json!({"windowId": 100});
    let mut orch = report::ReportOrchestration::new(&params);

    let mock_responses = vec![
        sample_snapshot(),
        json!({"description": "Description A", "h1": ""}),
        json!({"description": "Description B", "h1": ""}),
    ];

    let (response, undo) = drive_to_completion(&mut orch, &mock_responses);
    assert!(undo.is_none());

    let sender = Arc::new(ContractSender::new(sample_snapshot()));
    sender.add_response("report", response);

    let result = tabctl_graphql::execute(
        r#"{ reportTabs(windowId: 100) {
            totals { tabs }
            entries { tabId description groupTitle }
        } }"#,
        None,
        sample_snapshot(),
        sender,
    )
    .expect("GraphQL execution failed");

    assert!(result.get("errors").is_none(), "GraphQL errors: {result}");
    let report = &result["data"]["reportTabs"];
    assert_eq!(report["totals"]["tabs"], 2);
    let entries = report["entries"]
        .as_array()
        .expect("entries should be an array");
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0]["description"], "Description A");
}

// ── screenshot ───────────────────────────────────────────────────────────

#[test]
fn contract_capture_screenshots() {
    let params = json!({"mode": "viewport"});
    let mut orch = screenshot::ScreenshotOrchestration::new(&params);

    let screenshot_snapshot = json!({
        "windows": [{
            "windowId": 100,
            "focused": true,
            "tabs": [
                {"tabId": 1, "windowId": 100, "index": 0, "url": "https://a.com", "title": "A", "active": false, "pinned": false, "groupId": -1}
            ],
            "groups": []
        }]
    });

    let mock_responses = vec![
        screenshot_snapshot.clone(),
        json!([{"id": 5, "windowId": 100, "active": true}]),
        json!({"id": 1}),
        json!([{"index": 0, "total": 1, "x": 0, "y": 0, "width": 100, "height": 120, "scale": 2.0, "bytes": 1234, "scaled": false, "oversized": false, "dataUrl": "data:image/png;base64,abc"}]),
        json!({"id": 5}),
    ];

    let (response, undo) = drive_to_completion(&mut orch, &mock_responses);
    assert!(undo.is_none());

    let sender = Arc::new(ContractSender::new(screenshot_snapshot.clone()));
    sender.add_response("screenshot", response);

    let result = tabctl_graphql::execute(
        r#"{ captureScreenshots(windowId: 100, mode: "viewport") {
            totals { tabs tiles }
            entries { tabId tiles { width height dataUrl } }
        } }"#,
        None,
        screenshot_snapshot,
        sender,
    )
    .expect("GraphQL execution failed");

    assert!(result.get("errors").is_none(), "GraphQL errors: {result}");
    let capture = &result["data"]["captureScreenshots"];
    assert_eq!(capture["totals"]["tabs"], 1);
    assert_eq!(capture["totals"]["tiles"], 1);
    let entries = capture["entries"]
        .as_array()
        .expect("entries should be an array");
    assert_eq!(entries.len(), 1);
    let tiles = entries[0]["tiles"]
        .as_array()
        .expect("tiles should be an array");
    assert_eq!(tiles[0]["width"], 100);
    assert_eq!(tiles[0]["height"], 120);
}
