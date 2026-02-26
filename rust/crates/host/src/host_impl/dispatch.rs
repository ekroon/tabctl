use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::process;
use std::sync::{Arc, Mutex};
use std::thread;
use tabctl_shared::{ProtocolError, RequestEnvelope, ResponseEnvelope};

use super::protocol::{
    base_response, host_version, log_line, read_native_message, write_native_message,
};
use super::state::{HostEffect, HostState};

const MAX_RESPONSE_BYTES: usize = 20 * 1024 * 1024;

pub(super) type ClientWriter = Arc<Mutex<Box<dyn Write + Send>>>;
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
        let _ = writeln!(guard, "{serialized}");
        let _ = guard.flush();
    }
}

fn dispatch_effect(effect: HostEffect, clients: &Clients, native_out: &Arc<Mutex<io::Stdout>>) {
    match effect {
        HostEffect::SendNative(message) => {
            if let Ok(mut out) = native_out.lock() {
                if let Err(err) = write_native_message(&mut *out, &message) {
                    log_line(&format!("native write failed: {err}"));
                }
            }
        }
        HostEffect::Respond { client_id, payload } => {
            let stream = clients
                .lock()
                .ok()
                .and_then(|map| map.get(&client_id).cloned());
            if let Some(stream) = stream {
                send_response(&stream, &payload);
            }
        }
    }
}

pub(super) fn start_native_reader(
    state: Arc<Mutex<HostState>>,
    clients: Clients,
    native_out: Arc<Mutex<io::Stdout>>,
) {
    thread::spawn(move || {
        let mut stdin = io::stdin().lock();
        loop {
            match read_native_message(&mut stdin) {
                Ok(Some(message)) => {
                    let effects = {
                        let Ok(mut guard) = state.lock() else {
                            continue;
                        };
                        guard.handle_native_message(message)
                    };
                    for effect in effects {
                        dispatch_effect(effect, &clients, &native_out);
                    }
                }
                Ok(None) => break,
                Err(err) => {
                    log_line(&format!("failed to read native message: {err}"));
                    break;
                }
            }
        }
        process::exit(0);
    });
}

pub(super) fn handle_client(
    client_id: u64,
    reader: Box<dyn Read + Send>,
    state: Arc<Mutex<HostState>>,
    clients: Clients,
    native_out: Arc<Mutex<io::Stdout>>,
    expected_auth_token: Option<Arc<String>>,
) {
    let mut reader = BufReader::new(reader);
    let mut line = String::new();

    loop {
        line.clear();
        let read = reader.read_line(&mut line).unwrap_or(0);
        if read == 0 {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

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
            dispatch_effect(effect, &clients, &native_out);
        }
    }

    let _ = clients.lock().map(|mut map| map.remove(&client_id));
}
