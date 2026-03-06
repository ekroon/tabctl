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
    snapshot: Value,
}

impl ContractSender {
    fn new(snapshot: Value) -> Self {
        Self {
            responses: Mutex::new(Vec::new()),
            snapshot,
        }
    }

    fn add_response(&self, action: &str, response: Value) {
        self.responses
            .lock()
            .unwrap()
            .push((action.to_string(), response));
    }
}

impl tabctl_graphql::CommandSender for ContractSender {
    fn send(&self, action: &str, _params: Value) -> Result<Value, String> {
        let responses = self.responses.lock().unwrap();
        for (a, r) in responses.iter() {
            if a == action {
                return Ok(r.clone());
            }
        }
        Err(format!("ContractSender: no response registered for action '{action}'"))
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
        // p:tab-query (verification)
        json!([{"id": 50, "groupId": 30}, {"id": 51, "groupId": 30}]),
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
    let skipped = open["skippedUrls"].as_array().expect("skippedUrls should be array");
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
