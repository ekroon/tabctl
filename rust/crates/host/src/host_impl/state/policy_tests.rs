use super::privacy_tests::{start, window};
use super::regressions::{assert_error, reply, request, Fixture};
use super::*;
use serde_json::json;

fn state(fixture: &Fixture) -> HostState {
    let mut state = fixture.state();
    state.policy_path = state.undo_log.with_file_name("policy.json");
    std::fs::write(
        &state.policy_path,
        json!({
            "protect":{"pinned":true,"groupTitles":["🔒"],"domains":["protected.example"]}
        })
        .to_string(),
    )
    .unwrap();
    state
}

fn assert_rejected(state: &HostState, effects: &[HostEffect]) {
    assert_error(effects, "Mutation blocked by protection policy");
    assert!(state.pending.is_empty());
    assert!(state.transactions.is_empty());
    assert!(
        !state.undo_log.exists(),
        "Admission must reject before mutation or recovery writes"
    );
}

#[test]
fn assignment_admits_destination_group_before_moving_any_source() {
    let fixture = Fixture::new();
    let mut state = state(&fixture);
    let mut destination = window(2, false);
    destination["groups"][0]["title"] = json!("Destination 🔒");
    let effects = start(
        &mut state,
        "group-assign",
        json!({"tabIds":[101],"groupId":21}),
        json!({"windows":[window(1,false),destination]}),
    );
    assert_rejected(&state, &effects);
}

#[test]
fn gather_checks_every_source_and_keeper_in_both_operation_orders() {
    for keeper in [false, true] {
        for reversed in [false, true] {
            let fixture = Fixture::new();
            let mut state = state(&fixture);
            let mut later = window(2, false);
            later["tabs"][usize::from(!keeper)]["url"] = json!("https://protected.example");
            let mut windows = vec![window(1, false), later];
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
}

#[test]
fn merge_and_new_window_group_moves_reject_later_protected_sources_up_front() {
    for action in ["merge-window", "move-group"] {
        for reversed in [false, true] {
            let fixture = Fixture::new();
            let mut state = state(&fixture);
            let mut source = window(1, false);
            source["tabs"][1]["url"] = json!("https://protected.example");
            let params = if action == "merge-window" {
                json!({"fromWindowId":1,"toWindowId":2})
            } else {
                source["tabs"][1]["groupId"] = json!(11);
                json!({"groupId":11,"newWindow":true})
            };
            if reversed {
                source["tabs"].as_array_mut().unwrap().reverse();
            }
            let effects = start(
                &mut state,
                action,
                params,
                json!({"windows":[source,window(2,false)]}),
            );
            assert_rejected(&state, &effects);
        }
    }
}

#[test]
fn opening_into_a_protected_existing_group_rejects_before_creating_tabs() {
    let fixture = Fixture::new();
    let mut state = state(&fixture);
    let mut destination = window(1, false);
    destination["groups"][0]["title"] = json!("Destination 🔒");
    let effects = start(
        &mut state,
        "open",
        json!({"windowId":1,"groupTitle":"Destination 🔒","urls":["https://created.example"]}),
        json!({"windows":[destination]}),
    );
    assert_rejected(&state, &effects);
}

#[test]
fn unrelated_protected_tabs_and_windows_do_not_block_allowed_plans() {
    for action in [
        "group-assign",
        "group-gather",
        "merge-window",
        "move-group",
        "open",
    ] {
        let fixture = Fixture::new();
        let mut state = state(&fixture);
        let mut source = window(1, false);
        let mut destination = window(2, false);
        destination["tabs"][1]["url"] = json!("https://protected.example");
        let mut unrelated = window(3, false);
        unrelated["groups"][0]["title"] = json!("Unrelated 🔒");
        let params = match action {
            "group-assign" => {
                source["tabs"][1]["url"] = json!("https://protected.example");
                json!({"tabIds":[101],"groupId":21})
            }
            "group-gather" => json!({"windowId":1}),
            "merge-window" => json!({"fromWindowId":1,"toWindowId":2}),
            "move-group" => {
                source["tabs"][1]["url"] = json!("https://protected.example");
                json!({"groupId":11,"targetWindowId":2})
            }
            _ => {
                source["tabs"][1]["url"] = json!("https://protected.example");
                source["groups"][1]["title"] = json!("Unrelated");
                json!({"windowId":1,"groupTitle":"Dev","urls":["https://created.example"]})
            }
        };
        let effects = start(
            &mut state,
            action,
            params,
            json!({"windows":[source,destination,unrelated]}),
        );
        assert!(
            matches!(effects.as_slice(), [HostEffect::SendNative(_)]),
            "{action}: {effects:?}"
        );
        assert!(
            state.undo_log.exists(),
            "Allowed normal mutation must have recovery"
        );
    }
}

#[test]
fn eligible_close_archive_and_dedupe_plans_continue_to_skip_protected_tabs() {
    for action in ["close", "archive", "analyze"] {
        let fixture = Fixture::new();
        let mut state = state(&fixture);
        let mut source = window(1, false);
        source["tabs"][1]["pinned"] = json!(true);
        let params = if action == "analyze" {
            source["tabs"][1]["url"] = source["tabs"][0]["url"].clone();
            source["tabs"].as_array_mut().unwrap().push(json!({
                "tabId":103,"index":2,"url":"https://normal.example/first","groupId":-1
            }));
            json!({"windowId":1,"dedupe":true,"confirmed":true})
        } else {
            json!({"windowId":1,"confirmed":true})
        };
        let effects = start(&mut state, action, params, json!({"windows":[source]}));
        let [HostEffect::SendNative(native)] = effects.as_slice() else {
            panic!("eligible filtered plan must remain allowed: {effects:?}")
        };
        if action == "close" {
            assert_eq!(native.params.as_ref().unwrap()["tabIds"], json!([101]));
        }
        if action == "analyze" {
            assert_eq!(native.params.as_ref().unwrap()["tabIds"], json!([103]));
        }
        if action == "archive" {
            assert_eq!(native.action.as_deref(), Some("p:window-create"));
        }
    }
}

#[test]
fn undo_remains_exempt_when_original_targets_become_protected() {
    let fixture = Fixture::new();
    let mut state = state(&fixture);
    let before = json!({"windows":[window(1,false)]});
    let effects = start(
        &mut state,
        "move-tab",
        json!({"tabId":101,"windowId":1,"index":4}),
        before.clone(),
    );
    let [HostEffect::SendNative(native)] = effects.as_slice() else {
        panic!("expected move")
    };
    let effects = state.handle_native_message(reply(&native.id, json!({})));
    let [HostEffect::Respond { payload, .. }] = effects.as_slice() else {
        panic!("expected completion")
    };
    assert!(payload.ok);
    let txid = payload.data.as_ref().unwrap()["txid"].clone();
    std::fs::write(
        &state.policy_path,
        json!({"protect":{"domains":["normal.example"]}}).to_string(),
    )
    .unwrap();
    let mut after = before;
    after["windows"][0]["tabs"][0]["index"] = json!(4);
    after["windows"][0]["tabs"][0]["groupId"] = json!(-1);
    let mut effects = state.handle_cli_request(1, request("undo", json!({"txid":txid})));
    for _ in 0..4 {
        let [HostEffect::SendNative(native)] = effects.as_slice() else {
            panic!("undo must be allowed: {effects:?}")
        };
        if native.action.as_deref() != Some("p:snapshot") {
            assert_eq!(native.action.as_deref(), Some("p:tab-move"));
            return;
        }
        effects = state.handle_native_message(reply(&native.id, after.clone()));
    }
    panic!("undo never dispatched restoration");
}
