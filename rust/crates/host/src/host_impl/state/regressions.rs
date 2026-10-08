use super::*;
use crate::host_impl::orchestrate::{OrchStep, Orchestration};
use serde_json::json;
use std::fs;

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/recovery-regressions")
            .join(create_id("test"));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn state(&self) -> HostState {
        HostState::new(self.0.join("undo.jsonl"), self.0.join("focus.db"), None)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn request(action: &str, params: Value) -> RequestEnvelope {
    RequestEnvelope {
        id: Some(create_id("request")),
        action: action.into(),
        params,
        auth_token: None,
    }
}

fn reply(id: &str, data: Value) -> NativeMessage {
    NativeMessage {
        id: id.into(),
        action: None,
        ok: Some(true),
        progress: None,
        params: None,
        data: Some(data),
        error: None,
    }
}

fn snapshot() -> Value {
    json!({"windows":[{"windowId":7,"tabs":[{"tabId":1,"url":"https://a.example","index":0}]}]})
}

fn begin_close(state: &mut HostState) -> (String, String) {
    let effects =
        state.handle_cli_request(1, request("close", json!({"tabIds":[1],"confirmed":true})));
    let [HostEffect::SendNative(first)] = effects.as_slice() else {
        panic!("expected snapshot")
    };
    let effects = state.handle_native_message(reply(&first.id, snapshot()));
    let [HostEffect::SendNative(remove)] = effects.as_slice() else {
        panic!("expected removal")
    };
    let txid = state.pending[&remove.id].txid.clone().unwrap();
    (remove.id.clone(), txid)
}

fn assert_error(effects: &[HostEffect], needle: &str) {
    let [HostEffect::Respond { payload, .. }] = effects else {
        panic!("must not dispatch browser work")
    };
    assert!(!payload.ok);
    assert!(
        payload.error.as_ref().unwrap().message.contains(needle),
        "{payload:?}"
    );
}

#[test]
fn active_original_cannot_be_undone_explicitly_or_via_latest() {
    let fixture = Fixture::new();
    let mut state = fixture.state();
    let (primitive, txid) = begin_close(&mut state);
    let original = fs::read(&state.undo_log).unwrap();
    assert_error(
        &state.handle_cli_request(2, request("undo", json!({"txid":txid}))),
        "active",
    );
    assert_error(
        &state.handle_cli_request(2, request("undo", json!({"latest":true}))),
        "not found",
    );
    assert_eq!(fs::read(&state.undo_log).unwrap(), original);
    assert!(state.transactions.contains_key(&txid));
    state.handle_native_message(reply(&primitive, json!({})));
    let effects = state.handle_cli_request(2, request("undo", json!({"txid":txid})));
    assert!(matches!(effects.as_slice(), [HostEffect::SendNative(_)]));
}

#[test]
fn latest_skips_live_original_but_orphaned_in_progress_is_recoverable() {
    let fixture = Fixture::new();
    let mut state = fixture.state();
    let old = json!({"txid":"old","status":"completed","undo":{"action":"restore","tabs":[]}});
    append_undo_record(&state.undo_log, old.as_object().unwrap()).unwrap();
    let (primitive, txid) = begin_close(&mut state);
    let effects = state.handle_cli_request(2, request("undo", json!({"latest":true})));
    let [HostEffect::SendNative(undo)] = effects.as_slice() else {
        panic!("expected older undo")
    };
    assert_eq!(state.pending[&undo.id].txid.as_deref(), Some("old"));
    state.handle_native_message(reply(&primitive, json!({})));
    assert!(!state.transactions.contains_key(&txid));

    let other = Fixture::new();
    let mut first_process = other.state();
    let (_, orphan) = begin_close(&mut first_process);
    drop(first_process);
    let mut restarted = other.state();
    let effects = restarted.handle_cli_request(2, request("undo", json!({"latest":true})));
    let [HostEffect::SendNative(undo)] = effects.as_slice() else {
        panic!("orphaned removal should be recoverable")
    };
    assert_eq!(
        restarted.pending[&undo.id].txid.as_deref(),
        Some(orphan.as_str())
    );
}

#[derive(Debug)]
struct CompletedUndo;

impl Orchestration for CompletedUndo {
    fn start(&mut self) -> OrchStep {
        self.step(Value::Null)
    }
    fn step(&mut self, _: Value) -> OrchStep {
        OrchStep::Complete {
            response: json!({"summary":{"restoredTabs":0}}),
            undo: None,
        }
    }
}

#[test]
fn undo_reply_cannot_acknowledge_or_clear_a_live_original_transaction() {
    for failure in [false, true] {
        let fixture = Fixture::new();
        let mut state = fixture.state();
        let (primitive, txid) = begin_close(&mut state);
        let original = state.transactions[&txid].record.clone();
        state.pending.insert(
            "undo-reply".into(),
            PendingRequest {
                client_id: 2,
                action: "undo".into(),
                request_id: Some("undo".into()),
                txid: Some(txid.clone()),
                created_at: now_ms(),
                orchestration: Some(Box::new(CompletedUndo)),
                primitive: Some("p:window-create".into()),
            },
        );
        let effects = if failure {
            vec![state
                .fail_pending_request("undo-reply", "Write failed".into(), None)
                .unwrap()]
        } else {
            state.handle_native_message(reply("undo-reply", json!({"id":99,"tabs":[{"id":77}]})))
        };
        assert_error(&effects, if failure { "Write failed" } else { "active" });
        assert_eq!(state.transactions[&txid].record, original);
        assert_eq!(
            read_undo_records(&state.undo_log)[0]["status"],
            "in_progress"
        );
        state.handle_native_message(reply(&primitive, json!({})));
        assert_eq!(read_undo_records(&state.undo_log)[0]["status"], "completed");
    }
}

#[test]
fn unresolved_original_creation_survives_restart_without_empty_undo_success() {
    for action in ["p:tab-create", "p:window-create"] {
        let fixture = Fixture::new();
        let mut state = fixture.state();
        let record = json!({"txid":"unknown-create","status":"in_progress",
            "inFlight":{"action":action,"params":{"url":"https://created.example"}},
            "undo":{"action":"restore","tabs":[],"cleanupTabs":[]}
        });
        append_undo_record(&state.undo_log, record.as_object().unwrap()).unwrap();
        let original = fs::read(&state.undo_log).unwrap();
        assert_error(
            &state.handle_cli_request(2, request("undo", json!({"txid":"unknown-create"}))),
            "uncertain",
        );
        assert_error(
            &state.handle_cli_request(2, request("undo", json!({"latest":true}))),
            "not found",
        );
        assert_eq!(fs::read(&state.undo_log).unwrap(), original);
        let effects = state.handle_cli_request(2, request("history", json!({})));
        let [HostEffect::Respond { payload, .. }] = effects.as_slice() else {
            panic!("expected history")
        };
        assert_eq!(
            payload.data.as_ref().unwrap()[0]["status"],
            "recovery_uncertain"
        );
    }
}

#[test]
fn corrupt_journal_blocks_browser_mutation_and_reports_history_error() {
    let fixture = Fixture::new();
    let mut state = fixture.state();
    let broken = b"{\"txid\":\"valid\"}\n{\"truncated\":";
    fs::write(&state.undo_log, broken).unwrap();
    let effects =
        state.handle_cli_request(1, request("close", json!({"tabIds":[1],"confirmed":true})));
    let [HostEffect::SendNative(first)] = effects.as_slice() else {
        panic!("expected snapshot")
    };
    assert_error(
        &state.handle_native_message(reply(&first.id, snapshot())),
        "journal",
    );
    assert!(state.pending.is_empty());
    assert_eq!(fs::read(&state.undo_log).unwrap(), broken);
    assert_error(
        &state.handle_cli_request(1, request("history", json!({}))),
        "journal",
    );
}

#[test]
fn journal_corruption_after_undo_start_blocks_cleanup_dispatch() {
    let fixture = Fixture::new();
    let mut state = fixture.state();
    let record = json!({"txid":"created","status":"completed",
        "undo":{"action":"restore","tabs":[],"cleanupTabs":[1]}});
    append_undo_record(&state.undo_log, record.as_object().unwrap()).unwrap();
    let effects = state.handle_cli_request(2, request("undo", json!({"txid":"created"})));
    let [HostEffect::SendNative(first)] = effects.as_slice() else {
        panic!("expected undo snapshot")
    };
    let broken = b"{\"truncated\":";
    fs::write(&state.undo_log, broken).unwrap();
    let effects = state.handle_native_message(reply(&first.id, snapshot()));
    let [HostEffect::SendNative(groups)] = effects.as_slice() else {
        panic!("expected read-only group reconciliation")
    };
    assert_eq!(groups.action.as_deref(), Some("p:snapshot"));
    assert_error(
        &state.handle_native_message(reply(&groups.id, snapshot())),
        "journal",
    );
    assert_eq!(fs::read(&state.undo_log).unwrap(), broken);
}

fn begin_creation(state: &mut HostState, new_window: bool) -> (String, String) {
    let effects = state.handle_cli_request(
        1,
        request(
            "open",
            json!({
                "urls":["https://created.example"],"windowId":7,"newWindow":new_window
            }),
        ),
    );
    let [HostEffect::SendNative(first)] = effects.as_slice() else {
        panic!("expected snapshot")
    };
    let effects = state.handle_native_message(reply(&first.id, snapshot()));
    let [HostEffect::SendNative(create)] = effects.as_slice() else {
        panic!("expected creation")
    };
    assert_eq!(
        create.action.as_deref(),
        Some(if new_window {
            "p:window-create"
        } else {
            "p:tab-create"
        })
    );
    let txid = state.pending[&create.id].txid.clone().unwrap();
    (create.id.clone(), txid)
}

#[test]
fn failed_creation_replies_preserve_evidence_and_disclose_uncertain_recovery() {
    for new_window in [false, true] {
        for failure in ["transport", "timeout", "api", "metadata"] {
            let fixture = Fixture::new();
            let mut state = fixture.state();
            let (primitive, txid) = begin_creation(&mut state, new_window);
            let before = read_undo_records(&state.undo_log)[0]["inFlight"].clone();
            let history = state.handle_cli_request(2, request("history", json!({})));
            let [HostEffect::Respond { payload, .. }] = history.as_slice() else {
                panic!("expected history")
            };
            assert_eq!(payload.data.as_ref().unwrap()[0]["status"], "in_progress");
            let effects = match failure {
                "transport" => {
                    state.fail_all_pending_requests("Native transport lost".into(), None)
                }
                "timeout" => {
                    state.pending.get_mut(&primitive).unwrap().created_at =
                        now_ms().saturating_sub(REQUEST_TIMEOUT_MS + 1);
                    state.collect_timed_out_requests()
                }
                "api" => state.handle_native_message(NativeMessage {
                    ok: Some(false),
                    error: Some(ProtocolError {
                        message: "Creation failed".into(),
                        hint: None,
                    }),
                    ..reply(&primitive, Value::Null)
                }),
                _ => state.handle_native_message(reply(&primitive, json!({}))),
            };
            let [HostEffect::Respond { payload, .. }] = effects.as_slice() else {
                panic!("expected failure response")
            };
            assert!(!payload.ok);
            assert_eq!(
                payload.data.as_ref().unwrap()["status"],
                "recovery_uncertain"
            );
            assert!(payload
                .error
                .as_ref()
                .unwrap()
                .hint
                .as_ref()
                .unwrap()
                .contains("automatic undo is blocked"));
            assert_eq!(read_undo_records(&state.undo_log)[0]["inFlight"], before);
            assert!(!state.transactions.contains_key(&txid));
            let mut restarted = fixture.state();
            assert_error(
                &restarted.handle_cli_request(2, request("undo", json!({"txid":txid}))),
                "uncertain",
            );
        }
    }
}
