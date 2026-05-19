use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::process;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tabctl_shared::{ProtocolError, RequestEnvelope, ResponseEnvelope};

use super::protocol::{
    base_response, host_version, log_line, read_native_message, trace_line, write_native_message,
};
use super::state::{HostEffect, HostState};

const MAX_RESPONSE_BYTES: usize = 20 * 1024 * 1024;

pub(super) type ClientWriter = Arc<Mutex<Box<dyn Write + Send>>>;
pub(super) type NativeWriter = Arc<Mutex<Box<dyn Write + Send>>>;
pub(super) type Clients = Arc<Mutex<HashMap<u64, ClientWriter>>>;

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

fn start_native_reader_with<R>(
    mut reader: R,
    state: Arc<Mutex<HostState>>,
    clients: Clients,
    native_out: NativeWriter,
    exit_on_eof: bool,
) where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        loop {
            match read_native_message(&mut reader) {
                Ok(Some(message)) => {
                    trace_line(&format!(
                        "recv native: id={} ok={} progress={} action={}",
                        message.id,
                        message.ok.unwrap_or(false),
                        message.progress.unwrap_or(false),
                        message.action.as_deref().unwrap_or("<none>")
                    ));
                    let effects = {
                        let Ok(mut guard) = state.lock() else {
                            continue;
                        };
                        guard.handle_native_message(message)
                    };
                    for effect in effects {
                        dispatch_effect(effect, &state, &clients, &native_out);
                    }
                }
                Ok(None) => break,
                Err(err) => {
                    log_line(&format!("failed to read native message: {err}"));
                    break;
                }
            }
        }
        if exit_on_eof {
            process::exit(0);
        }
    });
}

pub(super) fn start_native_reader(
    state: Arc<Mutex<HostState>>,
    clients: Clients,
    native_out: NativeWriter,
) {
    start_native_reader_with(io::stdin(), state, clients, native_out, true);
}

pub(super) fn handle_client(
    client_id: u64,
    reader: Box<dyn Read + Send>,
    state: Arc<Mutex<HostState>>,
    clients: Clients,
    native_out: NativeWriter,
    expected_auth_token: Option<Arc<String>>,
    close_after_first_request: bool,
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

        trace_line(&format!(
            "client request: client_id={client_id} line={trimmed}"
        ));
        saw_request = true;
        let request = serde_json::from_str::<RequestEnvelope>(trimmed);
        let effects = match request {
            Ok(request) => {
                if let Some(ref expected) = expected_auth_token {
                    let provided = request.auth_token.as_deref().unwrap_or("");
                    if provided != expected.as_str() {
                        vec![HostEffect::Respond {
                            client_id,
                            payload: ResponseEnvelope {
                                ok: false,
                                action: None,
                                request_id: request.id,
                                component: Some("host".to_string()),
                                version: Some(host_version().to_string()),
                                progress: None,
                                data: None,
                                error: Some(ProtocolError {
                                    message: "Authentication failed".to_string(),
                                    hint: Some(
                                        "Invalid or missing auth token for TCP connection"
                                            .to_string(),
                                    ),
                                }),
                            },
                        }]
                    } else {
                        let Ok(mut guard) = state.lock() else {
                            continue;
                        };
                        guard.handle_cli_request(client_id, request)
                    }
                } else {
                    let Ok(mut guard) = state.lock() else {
                        continue;
                    };
                    guard.handle_cli_request(client_id, request)
                }
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

        if close_after_first_request {
            break;
        }
    }

    if !saw_request {
        let _ = clients.lock().map(|mut map| map.remove(&client_id));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host_impl::protocol::read_native_message;
    use std::io::{Cursor, Result as IoResult};
    use std::net::{TcpListener, TcpStream};
    use std::path::PathBuf;
    use std::time::{Duration, Instant};
    use tabctl_shared::{NativeMessage, ResponseEnvelope};

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

    fn test_state(native_channel_available: bool) -> Arc<Mutex<HostState>> {
        Arc::new(Mutex::new(HostState::new_with_native_channel(
            PathBuf::from("dispatch-test-undo.jsonl"),
            PathBuf::from("dispatch-test-focus.json"),
            None,
            native_channel_available,
        )))
    }

    fn native_sink() -> (NativeWriter, Arc<Mutex<Vec<u8>>>) {
        let sink = Arc::new(Mutex::new(Vec::new()));
        let writer: NativeWriter = Arc::new(Mutex::new(Box::new(SharedBufferWriter(sink.clone()))));
        (writer, sink)
    }

    #[test]
    fn handle_client_keeps_writer_registered_after_request_eof_for_async_response() {
        let state = test_state(true);
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
            None,
            false,
        );

        assert!(
            clients.lock().unwrap().contains_key(&client_id),
            "client writer should remain registered for async response delivery"
        );
    }

    #[test]
    fn handle_client_stops_after_first_request_when_requested() {
        let state = test_state(true);
        let clients: Clients = Arc::new(Mutex::new(HashMap::new()));
        let sink = Arc::new(Mutex::new(Vec::new()));
        let writer: ClientWriter = Arc::new(Mutex::new(Box::new(SharedBufferWriter(sink))));
        let client_id = 43;
        clients.lock().unwrap().insert(client_id, writer);

        let first = r#"{"id":"req-43","action":"snapshot","params":{}}"#;
        let second = r#"{"id":"req-44","action":"snapshot","params":{}}"#;
        let (native_out, native_sink) = native_sink();
        handle_client(
            client_id,
            Box::new(Cursor::new(format!("{first}\n{second}\n").into_bytes())),
            state,
            clients.clone(),
            native_out,
            None,
            true,
        );

        let mut cursor = Cursor::new(native_sink.lock().unwrap().clone());
        let native_request = read_native_message(&mut cursor)
            .expect("decode first native request")
            .expect("first native request exists");
        assert_eq!(native_request.action.as_deref(), Some("p:snapshot"));
        assert!(
            read_native_message(&mut cursor)
                .expect("decode remaining native requests")
                .is_none(),
            "close-after-first-request should stop before reading another request"
        );
        assert!(
            clients.lock().unwrap().contains_key(&client_id),
            "client writer should remain registered for async response delivery"
        );
    }

    #[test]
    fn handle_client_removes_writer_when_no_request_was_read() {
        let state = test_state(true);
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
            None,
            false,
        );

        assert!(
            !clients.lock().unwrap().contains_key(&client_id),
            "idle client without a request should be cleaned up"
        );
    }

    #[test]
    fn final_response_removes_client_after_write() {
        let state = test_state(true);
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

    #[test]
    fn end_to_end_async_native_response_is_returned_to_client() {
        let state = test_state(true);
        let clients: Clients = Arc::new(Mutex::new(HashMap::new()));
        let client_sink = Arc::new(Mutex::new(Vec::new()));
        let client_writer: ClientWriter = Arc::new(Mutex::new(Box::new(SharedBufferWriter(
            client_sink.clone(),
        ))));
        let client_id = 15;
        clients.lock().unwrap().insert(client_id, client_writer);

        let (native_out, native_sink) = native_sink();
        let request = r#"{"id":"req-15","action":"snapshot","params":{}}"#;
        handle_client(
            client_id,
            Box::new(Cursor::new(format!("{request}\n").into_bytes())),
            state.clone(),
            clients.clone(),
            native_out.clone(),
            None,
            false,
        );

        let native_bytes = native_sink.lock().unwrap().clone();
        let native_request = read_native_message(&mut Cursor::new(native_bytes))
            .expect("decode native request")
            .expect("native request exists");
        assert_eq!(native_request.action.as_deref(), Some("p:snapshot"));

        let effects = state.lock().unwrap().handle_native_message(NativeMessage {
            id: native_request.id,
            action: Some("p:snapshot".to_string()),
            ok: Some(true),
            progress: None,
            params: None,
            data: Some(serde_json::json!({
                "windows": [],
                "generatedAt": 1700000000000_u64
            })),
            error: None,
        });
        for effect in effects {
            dispatch_effect(effect, &state, &clients, &native_out);
        }

        let output = String::from_utf8(client_sink.lock().unwrap().clone()).unwrap();
        assert!(
            output.contains("\"ok\":true"),
            "missing success response: {output}"
        );
        assert!(
            output.contains("\"requestId\":\"req-15\""),
            "missing request id in response: {output}"
        );
        assert!(
            output.contains("\"action\":\"snapshot\""),
            "missing action in response: {output}"
        );
        assert!(
            !clients.lock().unwrap().contains_key(&client_id),
            "client should be removed after final async response"
        );
    }

    #[test]
    fn native_reader_thread_returns_async_response_to_client_end_to_end() {
        let state = test_state(true);
        let clients: Clients = Arc::new(Mutex::new(HashMap::new()));
        let client_sink = Arc::new(Mutex::new(Vec::new()));
        let client_writer: ClientWriter = Arc::new(Mutex::new(Box::new(SharedBufferWriter(
            client_sink.clone(),
        ))));
        let client_id = 21;
        clients.lock().unwrap().insert(client_id, client_writer);

        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind fake native listener");
        let addr = listener.local_addr().expect("fake native addr");
        let extension_side = TcpStream::connect(addr).expect("connect fake native client");
        let (host_side, _) = listener.accept().expect("accept fake native host side");
        let host_reader = host_side.try_clone().expect("clone fake native host side");
        let native_out: NativeWriter = Arc::new(Mutex::new(Box::new(host_side)));

        start_native_reader_with(
            host_reader,
            state.clone(),
            clients.clone(),
            native_out.clone(),
            false,
        );

        let request = r#"{"id":"req-21","action":"snapshot","params":{}}"#;
        handle_client(
            client_id,
            Box::new(Cursor::new(format!("{request}\n").into_bytes())),
            state,
            clients.clone(),
            native_out,
            None,
            false,
        );

        let mut extension_reader = extension_side.try_clone().expect("clone extension stream");
        let native_request = read_native_message(&mut extension_reader)
            .expect("read native request")
            .expect("native request exists");
        assert_eq!(native_request.action.as_deref(), Some("p:snapshot"));

        let mut extension_writer = extension_side;
        write_native_message(
            &mut extension_writer,
            &NativeMessage {
                id: native_request.id,
                action: Some("p:snapshot".to_string()),
                ok: Some(true),
                progress: None,
                params: None,
                data: Some(serde_json::json!({
                    "windows": [],
                    "generatedAt": 1700000001234_u64
                })),
                error: None,
            },
        )
        .expect("write native response");

        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            let output = String::from_utf8(client_sink.lock().unwrap().clone()).unwrap();
            if output.contains("\"requestId\":\"req-21\"") {
                assert!(
                    output.contains("\"ok\":true"),
                    "missing success response: {output}"
                );
                assert!(
                    output.contains("\"action\":\"snapshot\""),
                    "missing action: {output}"
                );
                assert!(
                    output.contains("\"generatedAt\":1700000001234"),
                    "missing data: {output}"
                );
                break;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for client response"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }
}
