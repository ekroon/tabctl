use super::regressions::{assert_error, reply, request, Fixture};
use super::*;
use serde_json::json;

pub(super) fn window(id: i64, private: bool) -> Value {
    let host = if private {
        "private.example"
    } else {
        "normal.example"
    };
    json!({
        "windowId":id,"incognito":private,"focused":true,
        "groups":[{"groupId":id*10+1,"title":"Dev"},{"groupId":id*10+2,"title":"Dev"}],
        "tabs":[
            {"tabId":id*100+1,"index":0,"groupId":id*10+1,"url":format!("https://{host}/first")},
            {"tabId":id*100+2,"index":1,"groupId":id*10+2,"url":format!("https://{host}/second")}
        ]
    })
}

pub(super) fn start(
    state: &mut HostState,
    action: &str,
    params: Value,
    snapshot: Value,
) -> Vec<HostEffect> {
    let effects = state.handle_cli_request(1, request(action, params));
    let [HostEffect::SendNative(native)] = effects.as_slice() else {
        panic!("expected initial snapshot: {effects:?}")
    };
    assert_eq!(native.action.as_deref(), Some("p:snapshot"));
    state.handle_native_message(reply(&native.id, snapshot))
}

fn assert_rejected(state: &HostState, effects: &[HostEffect]) {
    assert_error(effects, "Cannot mix private and regular");
    assert!(state.pending.is_empty());
    assert!(state.transactions.is_empty());
    assert!(
        !state.undo_log.exists(),
        "Rejected scope must not write recovery data"
    );
}

fn complete(state: &mut HostState, mut effects: Vec<HostEffect>, snapshot: &Value) -> Value {
    for _ in 0..32 {
        match effects.as_slice() {
            [HostEffect::Respond { payload, .. }] => {
                assert!(payload.ok, "{payload:?}");
                return payload.data.clone().unwrap();
            }
            [HostEffect::SendNative(native)] => {
                let response = match native.action.as_deref() {
                    Some("p:snapshot") => snapshot.clone(),
                    Some("p:window-create") => json!({"id":9,"tabs":[{"id":901}]}),
                    Some("p:tab-create") => {
                        json!({"id":999,"windowId":1,"url":"https://opened.example"})
                    }
                    Some("p:tab-group") => json!({"groupId":90}),
                    _ => json!({}),
                };
                effects = state.handle_native_message(reply(&native.id, response));
            }
            _ => panic!("unexpected effects: {effects:?}"),
        }
    }
    panic!("orchestration did not complete");
}

fn assert_privacy_result(state: &HostState, data: &Value, private: bool) {
    if private {
        assert!(data["txid"].is_null());
        assert_eq!(data["undoUnavailable"], "private tabs are never persisted");
        assert!(!state.undo_log.exists());
    } else {
        assert!(data["txid"].is_string());
        assert!(data.get("undoUnavailable").is_none());
        let records = read_undo_records(&state.undo_log);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0]["status"], "completed");
        assert!(!std::fs::read_to_string(&state.undo_log)
            .unwrap()
            .contains("private.example"));
    }
}

#[test]
fn gather_rejects_both_mixed_orders_before_any_mutation() {
    for reversed in [false, true] {
        let fixture = Fixture::new();
        let mut state = fixture.state();
        let mut windows = vec![window(1, false), window(2, true)];
        if reversed {
            windows.reverse();
        }
        let effects = start(
            &mut state,
            "group-gather",
            json!({}),
            json!({"windows":windows}),
        );
        assert_rejected(&state, &effects);
    }
}

#[test]
fn merge_rejects_mixed_source_and_destination_even_when_destination_empty() {
    for source_private in [false, true] {
        for empty_destination in [false, true] {
            let fixture = Fixture::new();
            let mut state = fixture.state();
            let mut destination = window(2, !source_private);
            if empty_destination {
                destination["tabs"] = json!([]);
                destination["groups"] = json!([]);
            }
            let effects = start(
                &mut state,
                "merge-window",
                json!({"fromWindowId":1,"toWindowId":2}),
                json!({"windows":[window(1, source_private),destination]}),
            );
            assert_rejected(&state, &effects);
        }
    }
}

#[test]
fn explicit_mixed_selections_reject_before_close_or_assignment() {
    for private_first in [false, true] {
        for action in ["close", "group-assign"] {
            let fixture = Fixture::new();
            let mut state = fixture.state();
            let ids = if private_first {
                json!([201, 101])
            } else {
                json!([101, 201])
            };
            let effects = start(
                &mut state,
                action,
                json!({"tabIds":ids,"confirmed":true,"groupId":11}),
                json!({"windows":[window(1,false),window(2,true)]}),
            );
            assert_rejected(&state, &effects);
        }
    }
}

#[test]
fn single_privacy_gather_and_merge_ignore_unrelated_opposite_privacy_windows() {
    for private in [false, true] {
        for action in ["group-gather", "merge-window"] {
            let fixture = Fixture::new();
            let mut state = fixture.state();
            let mut unrelated = window(3, !private);
            unrelated["groups"].as_array_mut().unwrap().pop();
            let snapshot = json!({"windows":[window(1,private),window(2,private),unrelated]});
            let params = if action == "merge-window" {
                json!({"fromWindowId":1,"toWindowId":2})
            } else {
                json!({})
            };
            let effects = start(&mut state, action, params, snapshot.clone());
            let data = complete(&mut state, effects, &snapshot);
            assert_privacy_result(&state, &data, private);
        }
    }
    let fixture = Fixture::new();
    let mut state = fixture.state();
    let snapshot = json!({"windows":[window(1,false),window(2,true)]});
    let effects = start(
        &mut state,
        "group-gather",
        json!({"windowId":1}),
        snapshot.clone(),
    );
    let data = complete(&mut state, effects, &snapshot);
    assert_privacy_result(&state, &data, false);
}

#[test]
fn private_only_open_archive_and_dedupe_expose_top_level_undo_unavailable() {
    for action in ["open", "archive", "analyze"] {
        let fixture = Fixture::new();
        let mut state = fixture.state();
        let mut private = window(1, true);
        let params = match action {
            "open" => json!({"windowId":1,"urls":["https://opened.example"]}),
            "analyze" => {
                for tab in private["tabs"].as_array_mut().unwrap() {
                    tab["url"] = json!("https://duplicate.example");
                }
                json!({"windowId":1,"dedupe":true,"confirmed":true})
            }
            _ => json!({"windowId":1}),
        };
        let snapshot = json!({"windows":[private,window(2,false)]});
        let effects = start(&mut state, action, params, snapshot.clone());
        let mut after = snapshot.clone();
        if action == "open" {
            after["windows"][0]["tabs"]
                .as_array_mut()
                .unwrap()
                .push(json!({
                    "tabId":999,"url":"https://opened.example","groupId":-1,"index":2
                }));
        }
        let data = complete(&mut state, effects, &after);
        assert_privacy_result(&state, &data, true);
    }
}

#[test]
fn archive_rejects_an_opposite_privacy_destination_before_moving_tabs() {
    for source_private in [false, true] {
        let fixture = Fixture::new();
        let mut state = fixture.state();
        let effects = start(
            &mut state,
            "archive",
            json!({"windowId":1,"archiveWindowId":2}),
            json!({"windows":[window(1,source_private),window(2,!source_private)]}),
        );
        assert_rejected(&state, &effects);
    }
}

#[test]
fn new_window_moves_preserve_private_scope_before_dispatch() {
    for action in ["move-tab", "move-group"] {
        let fixture = Fixture::new();
        let mut state = fixture.state();
        let effects = start(
            &mut state,
            action,
            json!({"tabId":101,"groupId":11,"newWindow":true}),
            json!({"windows":[window(1,true),window(2,false)]}),
        );
        let [HostEffect::SendNative(native)] = effects.as_slice() else {
            panic!("expected a private new-window move: {effects:?}")
        };
        assert_eq!(native.action.as_deref(), Some("p:window-create"));
        assert_eq!(native.params.as_ref().unwrap()["incognito"], true);
        assert!(!state.undo_log.exists());
    }
}
