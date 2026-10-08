//! Browser primitives are internal: external clients cannot bypass policy and undo.

mod common;

use common::*;
use serde_json::json;

#[test]
#[ignore = "requires built dist artifacts and Chrome or Edge"]
fn external_primitives_cannot_bypass_mutation_safety() {
    let browser = shared_browser();
    let before = browser.run_query("query { tabs(limit: 100) { items { tabId windowId } } }");
    for (action, params) in [
        ("p:snapshot", json!({})),
        ("p:tab-remove", json!({"tabIds":[]})),
        ("p:window-remove", json!({"windowId":2147483647})),
    ] {
        let response = browser.send_host_request(action, params);
        assert_eq!(response["ok"], false, "{response}");
        assert_eq!(
            response["error"]["message"],
            "Browser primitives are internal-only"
        );
    }
    let after = browser.run_query("query { tabs(limit: 100) { items { tabId windowId } } }");
    assert_eq!(after, before);
}
