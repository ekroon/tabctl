use super::*;

#[test]
fn in_flight_close_recreates_only_missing_targets() {
    for (exists, expected) in [(true, "p:tab-move"), (false, "p:tab-create")] {
        let mut orch = UndoOrchestration::new(&json!({"record":{"undo":{
            "action":"restore","tabs":[{"tabId":1,"url":"https://a.example","createIfMissing":true,
                "from":{"windowId":7,"index":0,"groupId":-1}}]
        }}}));
        let _ = orch.start();
        let tabs = if exists {
            json!([{"tabId":1,"index":2}])
        } else {
            json!([])
        };
        let step = orch.step(json!({"windows":[{"windowId":7,"tabs":tabs}]}));
        let OrchStep::SendPrimitive { action, params } = step else {
            panic!("expected restoration")
        };
        assert_eq!(action, expected);
        assert_eq!(params["index"], 0);
    }
}
