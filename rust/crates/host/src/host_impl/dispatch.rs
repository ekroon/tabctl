use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::process;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tabctl_shared::{NativeMessage, ProtocolError, RequestEnvelope, ResponseEnvelope};

use super::protocol::{
    base_response, host_version, log_line, read_native_message, trace_line, write_native_message,
};
use super::state::{HostEffect, HostState};

const MAX_RESPONSE_BYTES: usize = 20 * 1024 * 1024;

pub(super) type ClientWriter = Arc<Mutex<Box<dyn Write + Send>>>;
pub(super) type NativeWriter = Arc<Mutex<Box<dyn Write + Send>>>;
pub(super) type Clients = Arc<Mutex<HashMap<u64, ClientWriter>>>;

fn request_trace_summary(client_id: u64, line: &str) -> String {
    match serde_json::from_str::<RequestEnvelope>(line) {
        Ok(request) => {
            let request_id = request.id.as_deref().unwrap_or("<none>");
            format!(
                "client request: client_id={client_id} id={request_id} action={}",
                request.action
            )
        }
        Err(_) => format!("client request: client_id={client_id} line=<invalid-json>"),
    }
}

fn trace_request_line(client_id: u64, line: &str) {
    trace_line(&request_trace_summary(client_id, line));
}

fn send_response(stream: &ClientWriter, payload: &ResponseEnvelope) {
    let Ok(serialized) = serde_json::to_string(payload) else {
        return;
    };

    if serialized.len() > MAX_RESPONSE_BYTES {
        let mut too_large =
            base_response(false, payload.action.clone(), payload.request_id.clone());
        too_large.error = Some(ProtocolError {
            message: "Response too large".to_string(),
            hint: Some("Reduce scope or use --out to write files.".to_string()),
        });
        if let Ok(line) = serde_json::to_string(&too_large) {
            if let Ok(mut guard) = stream.lock() {
                let _ = writeln!(guard, "{line}");
                let _ = guard.flush();
            }
        }
        return;
    }

    if let Ok(mut guard) = stream.lock() {
        if let Err(err) = writeln!(guard, "{serialized}") {
            log_line(&format!("client write failed: {err}"));
            return;
        }
        if let Err(err) = guard.flush() {
            log_line(&format!("client flush failed: {err}"));
        }
    }
}

fn dispatch_effect(
    effect: HostEffect,
    state: &Arc<Mutex<HostState>>,
    clients: &Clients,
    native_out: &NativeWriter,
) {
    match effect {
        HostEffect::SendNative(message) => {
            trace_line(&format!(
                "send native: id={} action={}",
                message.id,
                message.action.as_deref().unwrap_or("<none>")
            ));
            if let Ok(mut out) = native_out.lock() {
                if let Err(err) = write_native_message(&mut *out, &message) {
                    log_line(&format!("native write failed: {err}"));
                    let follow_up = {
                        let Ok(mut guard) = state.lock() else {
                            return;
                        };
                        guard.fail_pending_request(
                            &message.id,
                            "Failed to write to native browser channel".to_string(),
                            Some(err.to_string()),
                        )
                    };
                    if let Some(effect) = follow_up {
                        dispatch_effect(effect, state, clients, native_out);
                    }
                }
            }
        }
        HostEffect::Respond { client_id, payload } => {
            trace_line(&format!(
                "respond client: client_id={} request_id={} action={} ok={} progress={}",
                client_id,
                payload.request_id.as_deref().unwrap_or("<none>"),
                payload.action.as_deref().unwrap_or("<none>"),
                payload.ok,
                payload.progress.unwrap_or(false)
            ));
            let is_final = !payload.progress.unwrap_or(false);
            let stream = clients
                .lock()
                .ok()
                .and_then(|map| map.get(&client_id).cloned());
            if let Some(stream) = stream {
                send_response(&stream, &payload);
                if is_final {
                    let _ = clients.lock().map(|mut map| map.remove(&client_id));
                }
            }
        }
    }
}

pub(super) fn start_request_timeout_reaper(
    state: Arc<Mutex<HostState>>,
    clients: Clients,
    native_out: NativeWriter,
) {
    thread::spawn(move || loop {
        thread::sleep(Duration::from_millis(250));
        let effects = {
            let Ok(mut guard) = state.lock() else {
                continue;
            };
            guard.collect_timed_out_requests()
        };
        for effect in effects {
            dispatch_effect(effect, &state, &clients, &native_out);
        }
    });
}

pub(super) fn start_native_reader(
    state: Arc<Mutex<HostState>>,
    clients: Clients,
    native_out: NativeWriter,
) {
    thread::spawn(move || {
        let mut reader = io::stdin();
        let outcome = read_native_messages(&mut reader, |message| {
            trace_line(&format!(
                "recv native: id={} ok={} progress={} action={}",
                message.id,
                message.ok.unwrap_or(false),
                message.progress.unwrap_or(false),
                message.action.as_deref().unwrap_or("<none>")
            ));
            let effects = {
                let Ok(mut guard) = state.lock() else {
                    return;
                };
                guard.handle_native_message(message)
            };
            for effect in effects {
                dispatch_effect(effect, &state, &clients, &native_out);
            }
        });
        let (message, hint) = match &outcome {
            Ok(()) => ("Native browser channel disconnected".to_string(), None),
            Err(err) => {
                log_line(&format!("failed to read native message: {err}"));
                (
                    "Invalid native browser message".to_string(),
                    Some(err.to_string()),
                )
            }
        };
        let effects = state
            .lock()
            .map(|mut guard| guard.fail_all_pending_requests(message, hint))
            .unwrap_or_default();
        for effect in effects {
            dispatch_effect(effect, &state, &clients, &native_out);
        }
        process::exit(i32::from(outcome.is_err()));
    });
}

fn read_native_messages(
    reader: &mut impl Read,
    mut receive: impl FnMut(NativeMessage),
) -> io::Result<()> {
    while let Some(message) = read_native_message(reader)? {
        receive(message);
    }
    Ok(())
}

pub(super) fn handle_client(
    client_id: u64,
    reader: Box<dyn Read + Send>,
    state: Arc<Mutex<HostState>>,
    clients: Clients,
    native_out: NativeWriter,
) {
    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    let mut saw_request = false;

    loop {
        line.clear();
        let read = match reader.read_line(&mut line) {
            Ok(read) => read,
            Err(err) => {
                log_line(&format!("client read error: {err}"));
                0
            }
        };
        trace_line(&format!("client read: client_id={client_id} bytes={read}"));
        if read == 0 {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        trace_request_line(client_id, trimmed);
        saw_request = true;
        let request = serde_json::from_str::<RequestEnvelope>(trimmed);
        let effects = match request {
            Ok(request) => {
                let Ok(mut guard) = state.lock() else {
                    continue;
                };
                guard.handle_cli_request(client_id, request)
            }
            Err(_) => {
                vec![HostEffect::Respond {
                    client_id,
                    payload: ResponseEnvelope {
                        ok: false,
                        action: None,
                        request_id: None,
                        component: Some("host".to_string()),
                        version: Some(host_version().to_string()),
                        progress: None,
                        data: None,
                        error: Some(ProtocolError {
                            message: "Invalid JSON".to_string(),
                            hint: None,
                        }),
                    },
                }]
            }
        };

        for effect in effects {
            dispatch_effect(effect, &state, &clients, &native_out);
        }
    }

    if !saw_request {
        let _ = clients.lock().map(|mut map| map.remove(&client_id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host_impl::protocol::{create_id, now_ms};
    use std::fs;
    use std::io::{Cursor, Result as IoResult};

    struct SharedBufferWriter(Arc<Mutex<Vec<u8>>>);

    impl Write for SharedBufferWriter {
        fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
            if let Ok(mut inner) = self.0.lock() {
                inner.extend_from_slice(buf);
            }
            Ok(buf.len())
        }

        fn flush(&mut self) -> IoResult<()> {
            Ok(())
        }
    }

    struct FixtureRoot(std::path::PathBuf);

    impl Drop for FixtureRoot {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).expect("clean dispatch fixture");
        }
    }

    fn test_state(native_channel_available: bool) -> (Arc<Mutex<HostState>>, FixtureRoot) {
        let base = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/dispatch-tests")
            .join(format!(
                "tabctl-host-dispatch-{}-{}",
                now_ms(),
                create_id("test")
            ));
        fs::create_dir_all(&base).expect("create isolated dispatch test fixture");
        let state = Arc::new(Mutex::new(HostState::new_with_native_channel(
            base.join("dispatch-test-undo.jsonl"),
            base.join("dispatch-test-focus.json"),
            None,
            native_channel_available,
        )));
        (state, FixtureRoot(base))
    }

    fn native_sink() -> (NativeWriter, Arc<Mutex<Vec<u8>>>) {
        let sink = Arc::new(Mutex::new(Vec::new()));
        let writer: NativeWriter = Arc::new(Mutex::new(Box::new(SharedBufferWriter(sink.clone()))));
        (writer, sink)
    }

    #[test]
    fn request_trace_summary_omits_auth_token() {
        let line = r#"{"id":"req-1","action":"ping","params":{},"authToken":"secret"}"#;

        let summary = request_trace_summary(7, line);

        assert_eq!(summary, "client request: client_id=7 id=req-1 action=ping");
        assert!(!summary.contains("secret"));
        assert!(!summary.contains("authToken"));
    }

    #[test]
    fn native_reader_distinguishes_clean_eof_from_invalid_or_oversized_input() {
        assert!(read_native_messages(&mut Cursor::new(Vec::<u8>::new()), |_| {}).is_ok());
        for input in [
            vec![1, 0],
            [1_u32.to_le_bytes().as_slice(), b"{".as_slice()].concat(),
            ((super::super::protocol::MAX_NATIVE_MESSAGE_BYTES + 1) as u32)
                .to_le_bytes()
                .to_vec(),
        ] {
            assert!(read_native_messages(&mut Cursor::new(input), |_| {
                panic!("invalid input must never be delivered")
            })
            .is_err());
        }
    }

    #[test]
    fn handle_client_keeps_writer_registered_after_request_eof_for_async_response() {
        let (state, _fixture) = test_state(true);
        let clients: Clients = Arc::new(Mutex::new(HashMap::new()));
        let sink = Arc::new(Mutex::new(Vec::new()));
        let writer: ClientWriter = Arc::new(Mutex::new(Box::new(SharedBufferWriter(sink))));
        let client_id = 42;
        clients.lock().unwrap().insert(client_id, writer);

        let request = r#"{"id":"req-1","action":"snapshot","params":{}}"#;
        let (native_out, _native_sink) = native_sink();
        handle_client(
            client_id,
            Box::new(Cursor::new(format!("{request}\n").into_bytes())),
            state,
            clients.clone(),
            native_out,
        );

        assert!(
            clients.lock().unwrap().contains_key(&client_id),
            "client writer should remain registered for async response delivery"
        );
    }

    #[test]
    fn handle_client_removes_writer_when_no_request_was_read() {
        let (state, _fixture) = test_state(true);
        let clients: Clients = Arc::new(Mutex::new(HashMap::new()));
        let sink = Arc::new(Mutex::new(Vec::new()));
        let writer: ClientWriter = Arc::new(Mutex::new(Box::new(SharedBufferWriter(sink))));
        let client_id = 7;
        clients.lock().unwrap().insert(client_id, writer);

        let (native_out, _native_sink) = native_sink();
        handle_client(
            client_id,
            Box::new(Cursor::new(Vec::<u8>::new())),
            state,
            clients.clone(),
            native_out,
        );

        assert!(
            !clients.lock().unwrap().contains_key(&client_id),
            "idle client without a request should be cleaned up"
        );
    }

    #[test]
    fn final_response_removes_client_after_write() {
        let (state, _fixture) = test_state(true);
        let clients: Clients = Arc::new(Mutex::new(HashMap::new()));
        let sink = Arc::new(Mutex::new(Vec::new()));
        let writer: ClientWriter = Arc::new(Mutex::new(Box::new(SharedBufferWriter(sink.clone()))));
        let client_id = 9;
        clients.lock().unwrap().insert(client_id, writer);

        let (native_out, _native_sink) = native_sink();
        dispatch_effect(
            HostEffect::Respond {
                client_id,
                payload: ResponseEnvelope {
                    ok: true,
                    action: Some("snapshot".to_string()),
                    request_id: Some("req-9".to_string()),
                    component: None,
                    version: None,
                    progress: None,
                    data: Some(serde_json::json!({"ok": true})),
                    error: None,
                },
            },
            &state,
            &clients,
            &native_out,
        );

        assert!(!clients.lock().unwrap().contains_key(&client_id));
        let output = String::from_utf8(sink.lock().unwrap().clone()).unwrap();
        assert!(output.contains("\"requestId\":\"req-9\""));
    }
}
