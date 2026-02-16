use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Deserialize)]
struct Request {
    id: Option<String>,
    action: String,
    #[serde(default)]
    params: Value,
}

fn write_native_message(payload: &Value) -> io::Result<()> {
    let json = serde_json::to_vec(payload)?;
    let len = (json.len() as u32).to_le_bytes();
    let stdout = io::stdout();
    let mut lock = stdout.lock();
    lock.write_all(&len)?;
    lock.write_all(&json)?;
    lock.flush()
}

fn read_native_message(input: &mut dyn Read) -> io::Result<Option<Vec<u8>>> {
    let mut len_buf = [0u8; 4];
    match input.read_exact(&mut len_buf) {
        Ok(()) => {
            let len = u32::from_le_bytes(len_buf) as usize;
            let mut payload = vec![0u8; len];
            input.read_exact(&mut payload)?;
            Ok(Some(payload))
        }
        Err(err) if err.kind() == io::ErrorKind::UnexpectedEof => Ok(None),
        Err(err) => Err(err),
    }
}

fn ok(action: &str, request_id: Option<&str>, data: Value) -> Value {
    json!({
        "ok": true,
        "action": action,
        "requestId": request_id,
        "component": "host-rust-mvp",
        "version": env!("CARGO_PKG_VERSION"),
        "data": data
    })
}

fn err(action: &str, request_id: Option<&str>, message: &str, hint: Option<&str>) -> Value {
    let mut error = json!({ "message": message });
    if let Some(hint_text) = hint {
        error["hint"] = Value::String(hint_text.to_string());
    }
    json!({
        "ok": false,
        "action": action,
        "requestId": request_id,
        "component": "host-rust-mvp",
        "version": env!("CARGO_PKG_VERSION"),
        "error": error
    })
}

fn main() -> io::Result<()> {
    let mut history: VecDeque<Value> = VecDeque::new();
    let stdin = io::stdin();
    let mut lock = stdin.lock();

    while let Some(payload) = read_native_message(&mut lock)? {
        let parsed: Result<Request, _> = serde_json::from_slice(&payload);
        let request = match parsed {
            Ok(req) => req,
            Err(_) => {
                write_native_message(&err(
                    "unknown",
                    None,
                    "Invalid JSON payload",
                    Some("Expected native messaging payload with JSON body"),
                ))?;
                continue;
            }
        };

        let request_id = request.id.as_deref();
        let response = match request.action.as_str() {
            "ping" => ok(
                "ping",
                request_id,
                json!({
                    "now": SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0),
                    "component": "host-rust-mvp",
                    "version": env!("CARGO_PKG_VERSION")
                }),
            ),
            "version" => ok(
                "version",
                request_id,
                json!({
                    "component": "host-rust-mvp",
                    "version": env!("CARGO_PKG_VERSION"),
                    "baseVersion": env!("CARGO_PKG_VERSION"),
                    "gitSha": Value::Null,
                    "dirty": false
                }),
            ),
            "history" => {
                let list: Vec<Value> = history.iter().cloned().collect();
                ok("history", request_id, Value::Array(list))
            }
            "undo" => {
                let txid = request.params.get("txid").and_then(Value::as_str);
                let latest = request
                    .params
                    .get("latest")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                if txid.is_none() && !latest {
                    err(
                        "undo",
                        request_id,
                        "Missing txid",
                        Some("Use txid or latest=true"),
                    )
                } else {
                    ok(
                        "undo",
                        request_id,
                        json!({
                            "status": "accepted",
                            "txid": txid,
                            "latest": latest,
                            "note": "MVP placeholder: extension forwarding not implemented yet"
                        }),
                    )
                }
            }
            forwarded => err(
                forwarded,
                request_id,
                "Action not implemented in Rust MVP",
                Some("Forwarding to extension will be added in next phase"),
            ),
        };

        if let Some(txid) = response
            .get("data")
            .and_then(|d| d.get("txid"))
            .and_then(Value::as_str)
        {
            history.push_back(json!({
                "txid": txid,
                "action": request.action,
            }));
            while history.len() > 100 {
                history.pop_front();
            }
        }

        write_native_message(&response)?;
    }

    Ok(())
}
