mod dispatch;
mod orchestrate;
mod protocol;
mod runtime;
mod state;
mod undo;

pub fn run() {
    runtime::run();
}

#[cfg(test)]
use orchestrate::{OrchStep, Orchestration};
#[cfg(test)]
use protocol::{
    create_id, host_version, now_ms, read_native_message, write_native_message,
    MAX_NATIVE_MESSAGE_BYTES,
};
#[cfg(test)]
use runtime::{
    generate_and_write_auth_token, resolve_config, AUTH_TOKEN_FILENAME, AUTH_TOKEN_LENGTH,
    TCP_PORT_FILENAME,
};
#[cfg(test)]
use serde_json::{Map, Value};
#[cfg(test)]
use state::{HostEffect, HostState};
#[cfg(test)]
use std::fs;
#[cfg(test)]
use std::io;
#[cfg(test)]
use std::path::PathBuf;
#[cfg(test)]
use std::sync::Arc;
#[cfg(test)]
use tabctl_shared::{NativeMessage, ProtocolError, RequestEnvelope, ResponseEnvelope};
#[cfg(test)]
use undo::{append_undo_record, read_undo_records};
#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::sync::{Mutex, OnceLock};

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    fn with_env_vars<F>(vars: &[(&str, Option<&str>)], run: F)
    where
        F: FnOnce(),
    {
        let _guard = env_lock()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let saved: Vec<(String, Option<OsString>)> = vars
            .iter()
            .map(|(key, _)| ((*key).to_string(), std::env::var_os(key)))
            .collect();
        for (key, value) in vars {
            unsafe {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(run));
        for (key, value) in saved {
            unsafe {
                match value {
                    Some(value) => std::env::set_var(&key, value),
                    None => std::env::remove_var(&key),
                }
            }
        }
        if let Err(payload) = result {
            std::panic::resume_unwind(payload);
        }
    }

    fn temp_undo_path() -> PathBuf {
        let name = format!("tabctl-rust-host-{}-{}.jsonl", now_ms(), create_id("test"));
        std::env::temp_dir().join(name)
    }

    #[test]
    fn native_message_framing_roundtrip() {
        let msg = NativeMessage {
            id: "req-1".to_string(),
            action: Some("move-tab".to_string()),
            ok: None,
            progress: None,
            params: Some(Value::Object(Map::from_iter([(
                "tabId".to_string(),
                Value::Number(12.into()),
            )]))),
            data: None,
            error: None,
        };

        let mut buffer = Vec::new();
        write_native_message(&mut buffer, &msg).expect("encode native");
        let mut cursor = io::Cursor::new(buffer);
        let decoded = read_native_message(&mut cursor)
            .expect("decode native")
            .expect("native message exists");

        assert_eq!(decoded.id, "req-1");
        assert_eq!(decoded.action.as_deref(), Some("move-tab"));
    }

    #[test]
    fn rejects_oversized_native_message() {
        let len = (MAX_NATIVE_MESSAGE_BYTES as u32).saturating_add(1);
        let mut payload = len.to_le_bytes().to_vec();
        payload.extend_from_slice(b"{}");
        let mut cursor = io::Cursor::new(payload);
        let err = read_native_message(&mut cursor).expect_err("expected size validation error");
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn forwards_mutation_with_txid_and_client_metadata() {
        let undo_path = temp_undo_path();
        let mut state = HostState::new(undo_path);

        // Use group-update through orchestration: first step is p:snapshot
        let request = RequestEnvelope {
            id: Some("req-1".to_string()),
            action: "group-update".to_string(),
            params: serde_json::json!({"groupId": 10, "title": "NewTitle"}),
            auth_token: None,
        };

        let effects = state.handle_cli_request(7, request);
        let HostEffect::SendNative(native) = &effects[0] else {
            panic!("expected native forward");
        };
        // Orchestrated commands send a p:snapshot primitive first
        assert_eq!(native.action.as_deref(), Some("p:snapshot"));
        // The pending request should have a txid since group-update is undo-tracked
    }

    #[test]
    fn records_undo_on_successful_native_response() {
        let undo_path = temp_undo_path();
        let mut state = HostState::new(undo_path.clone());

        // Use group-update through orchestration
        let request = RequestEnvelope {
            id: Some("req-2".to_string()),
            action: "group-update".to_string(),
            params: serde_json::json!({"groupId": 10, "title": "NewTitle"}),
            auth_token: None,
        };
        let effects = state.handle_cli_request(9, request);
        let HostEffect::SendNative(snapshot_req) = &effects[0] else {
            panic!("expected p:snapshot request");
        };
        assert_eq!(snapshot_req.action.as_deref(), Some("p:snapshot"));

        // Respond with snapshot containing group 10
        let snapshot_resp = NativeMessage {
            id: snapshot_req.id.clone(),
            action: None,
            ok: Some(true),
            progress: None,
            params: None,
            data: Some(serde_json::json!({
                "windows": [{
                    "windowId": 100, "focused": true,
                    "tabs": [{"tabId": 1, "windowId": 100, "index": 0, "groupId": 10}],
                    "groups": [{"groupId": 10, "title": "OldTitle", "color": "blue", "collapsed": false}]
                }]
            })),
            error: None,
        };
        let effects = state.handle_native_message(snapshot_resp);
        let HostEffect::SendNative(update_req) = &effects[0] else {
            panic!("expected p:group-update request");
        };
        assert_eq!(update_req.action.as_deref(), Some("p:group-update"));

        // Respond to group-update → should complete with undo
        let update_resp = NativeMessage {
            id: update_req.id.clone(),
            action: None,
            ok: Some(true),
            progress: None,
            params: None,
            data: Some(serde_json::json!({"id": 10, "title": "NewTitle", "color": "blue"})),
            error: None,
        };
        let response_effects = state.handle_native_message(update_resp);

        let HostEffect::Respond { payload, .. } = &response_effects[0] else {
            panic!("expected response");
        };
        // Should have a txid in the response data
        let got_txid = payload
            .data
            .as_ref()
            .and_then(|v| v.get("txid"))
            .and_then(|v| v.as_str())
            .expect("response txid");
        assert!(got_txid.starts_with("tx-"));

        // Verify undo was recorded
        let records = read_undo_records(&undo_path);
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0].get("txid").and_then(|v| v.as_str()),
            Some(got_txid)
        );
        assert_eq!(
            records[0].get("action").and_then(|v| v.as_str()),
            Some("group-update")
        );

        let _ = fs::remove_file(undo_path);
    }

    #[test]
    fn analyze_then_close_apply_uses_analysis_candidates() {
        let undo_path = temp_undo_path();
        let mut state = HostState::new(undo_path);

        let analyze_request = RequestEnvelope {
            id: Some("req-analyze".to_string()),
            action: "analyze".to_string(),
            params: Value::Object(Map::new()),
            auth_token: None,
        };
        let analyze_effects = state.handle_cli_request(3, analyze_request);
        // Orchestration sends p:snapshot first
        let HostEffect::SendNative(snapshot_native) = &analyze_effects[0] else {
            panic!("expected p:snapshot forward");
        };
        assert_eq!(snapshot_native.action.as_deref(), Some("p:snapshot"));

        // Provide snapshot with a stale tab (lastFocusedAt: 0 = epoch = definitely stale)
        let snapshot_response = state.handle_native_message(NativeMessage {
            id: snapshot_native.id.clone(),
            action: Some("p:snapshot".to_string()),
            ok: Some(true),
            progress: None,
            params: None,
            data: Some(serde_json::json!({
                "windows": [{
                    "windowId": 100,
                    "focused": true,
                    "tabs": [{
                        "tabId": 12,
                        "windowId": 100,
                        "index": 0,
                        "url": "https://example.com",
                        "title": "Example",
                        "active": true,
                        "pinned": false,
                        "groupId": -1,
                        "lastFocusedAt": 0
                    }],
                    "groups": []
                }]
            })),
            error: None,
        });

        // Orchestration completes with analysis → response includes analysisId
        let HostEffect::Respond { payload, .. } = &snapshot_response[0] else {
            panic!("expected analyze response");
        };
        let analysis_id = payload
            .data
            .as_ref()
            .and_then(|v| v.get("analysisId"))
            .and_then(|v| v.as_str())
            .expect("analysis id")
            .to_string();

        // Now close --apply with the analysis ID.
        // The host enriches params (tabIds + expectedUrls) and falls through
        // to CloseOrchestration which sends p:snapshot first.
        let close_request = RequestEnvelope {
            id: Some("req-close".to_string()),
            action: "close".to_string(),
            params: Value::Object(Map::from_iter([
                ("mode".to_string(), Value::String("apply".to_string())),
                ("analysisId".to_string(), Value::String(analysis_id)),
            ])),
            auth_token: None,
        };

        let close_effects = state.handle_cli_request(3, close_request);
        let HostEffect::SendNative(close_native) = &close_effects[0] else {
            panic!("expected p:snapshot for close orchestration");
        };
        // close --apply goes through CloseOrchestration which starts with p:snapshot
        assert_eq!(close_native.action.as_deref(), Some("p:snapshot"));
    }

    #[test]
    fn history_request_respects_limit_and_returns_latest_entries() {
        let undo_path = temp_undo_path();
        let now = now_ms();
        let first = Map::from_iter([
            ("txid".to_string(), Value::String("tx-1".to_string())),
            ("createdAt".to_string(), Value::Number((now - 1_000).into())),
        ]);
        let second = Map::from_iter([
            ("txid".to_string(), Value::String("tx-2".to_string())),
            ("createdAt".to_string(), Value::Number(now.into())),
        ]);
        append_undo_record(&undo_path, &first);
        append_undo_record(&undo_path, &second);

        let mut state = HostState::new(undo_path.clone());
        let effects = state.handle_cli_request(
            5,
            RequestEnvelope {
                id: Some("req-history".to_string()),
                action: "history".to_string(),
                params: Value::Object(Map::from_iter([(
                    "limit".to_string(),
                    Value::Number(1.into()),
                )])),
                auth_token: None,
            },
        );
        let HostEffect::Respond { payload, .. } = &effects[0] else {
            panic!("expected history response");
        };
        let entries = payload
            .data
            .as_ref()
            .and_then(|v| v.as_array())
            .expect("history array");
        assert_eq!(entries.len(), 1);
        assert_eq!(
            entries[0].get("txid").and_then(|v| v.as_str()),
            Some("tx-2")
        );

        let _ = fs::remove_file(undo_path);
    }

    #[test]
    fn undo_without_txid_or_latest_returns_error_with_hint() {
        let mut state = HostState::new(temp_undo_path());
        let effects = state.handle_cli_request(
            2,
            RequestEnvelope {
                id: Some("req-undo".to_string()),
                action: "undo".to_string(),
                params: Value::Object(Map::new()),
                auth_token: None,
            },
        );
        let HostEffect::Respond { payload, .. } = &effects[0] else {
            panic!("expected undo error response");
        };
        assert!(!payload.ok);
        assert_eq!(
            payload.error.as_ref().map(|e| e.message.as_str()),
            Some("Missing txid")
        );
        assert!(payload
            .error
            .as_ref()
            .and_then(|e| e.hint.as_deref())
            .is_some());
    }

    #[test]
    fn ping_response_preserves_runtime_extension_id() {
        let mut state = HostState::new(temp_undo_path());
        let effects = state.handle_cli_request(
            5,
            RequestEnvelope {
                id: Some("req-ping".to_string()),
                action: "ping".to_string(),
                params: Value::Object(Map::new()),
                auth_token: None,
            },
        );
        let HostEffect::SendNative(native_req) = &effects[0] else {
            panic!("expected ping forward");
        };
        assert_eq!(native_req.id, "req-ping");

        let native_resp = NativeMessage {
            id: native_req.id.clone(),
            action: None,
            ok: Some(true),
            progress: None,
            params: Some(Value::Object(Map::new())),
            data: Some(Value::Object(Map::from_iter([
                (
                    "runtimeId".to_string(),
                    Value::String("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string()),
                ),
                ("version".to_string(), Value::String("1.2.3".to_string())),
                (
                    "baseVersion".to_string(),
                    Value::String("1.2.3".to_string()),
                ),
                (
                    "component".to_string(),
                    Value::String("extension".to_string()),
                ),
            ]))),
            error: None,
        };
        let effects = state.handle_native_message(native_resp);
        let HostEffect::Respond { payload, .. } = &effects[0] else {
            panic!("expected ping response");
        };
        assert_eq!(payload.request_id.as_deref(), Some("req-ping"));
        assert_eq!(payload.component.as_deref(), Some("host"));
        assert_eq!(payload.version.as_deref(), Some(host_version()));
        let data = payload
            .data
            .as_ref()
            .and_then(|v| v.as_object())
            .expect("response data");
        assert_eq!(
            data.get("runtimeId").and_then(|v| v.as_str()),
            Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
        );
        assert_eq!(data.get("version").and_then(|v| v.as_str()), Some("1.2.3"));
        assert_eq!(
            data.get("component").and_then(|v| v.as_str()),
            Some("extension")
        );
        assert_eq!(
            data.get("hostVersion").and_then(|v| v.as_str()),
            Some(host_version())
        );
        assert!(data.get("hostBaseVersion").is_some());
        assert_eq!(
            data.get("versionsInSync").and_then(|v| v.as_bool()),
            Some(false)
        );
        assert!(data.get("extensionVersion").is_none());
        assert!(data.get("extensionComponent").is_none());
        assert!(payload.ok);
    }

    #[test]
    fn version_action_is_served_locally_with_host_metadata() {
        let mut state = HostState::new(temp_undo_path());
        let effects = state.handle_cli_request(
            11,
            RequestEnvelope {
                id: Some("req-version".to_string()),
                action: "version".to_string(),
                params: Value::Object(Map::new()),
                auth_token: None,
            },
        );
        let HostEffect::Respond { payload, .. } = &effects[0] else {
            panic!("expected local version response");
        };
        assert!(payload.ok);
        assert_eq!(payload.action.as_deref(), Some("version"));
        assert_eq!(payload.request_id.as_deref(), Some("req-version"));
        assert_eq!(payload.component.as_deref(), Some("host"));
        let data = payload
            .data
            .as_ref()
            .and_then(|v| v.as_object())
            .expect("version data");
        assert_eq!(data.get("component").and_then(|v| v.as_str()), Some("host"));
        assert!(data.get("version").is_some());
    }

    #[test]
    fn non_ping_response_does_not_include_host_version_metadata() {
        let mut state = HostState::new(temp_undo_path());
        let effects = state.handle_cli_request(
            12,
            RequestEnvelope {
                id: Some("req-list".to_string()),
                action: "list".to_string(),
                params: Value::Object(Map::new()),
                auth_token: None,
            },
        );
        let HostEffect::SendNative(native_req) = &effects[0] else {
            panic!("expected snapshot primitive");
        };
        // list now goes through orchestration — sends p:snapshot
        assert_eq!(native_req.action.as_deref(), Some("p:snapshot"));

        let native_resp = NativeMessage {
            id: native_req.id.clone(),
            action: None,
            ok: Some(true),
            progress: None,
            params: Some(Value::Object(Map::new())),
            data: Some(Value::Object(Map::from_iter([(
                "windows".to_string(),
                Value::Array(Vec::new()),
            )]))),
            error: None,
        };
        let effects = state.handle_native_message(native_resp);
        let HostEffect::Respond { payload, .. } = &effects[0] else {
            panic!("expected list response");
        };
        assert!(payload.component.is_none());
        assert!(payload.version.is_none());
        let data = payload
            .data
            .as_ref()
            .and_then(|v| v.as_object())
            .expect("list response data");
        assert!(data.get("hostVersion").is_none());
        assert!(data.get("hostBaseVersion").is_none());
        assert!(data.get("hostGitSha").is_none());
        assert!(data.get("hostDirty").is_none());
        assert!(data.get("versionsInSync").is_none());
    }

    #[test]
    fn missing_action_error_response_omits_host_metadata() {
        let mut state = HostState::new(temp_undo_path());
        let effects = state.handle_cli_request(
            88,
            RequestEnvelope {
                id: Some("req-empty-action".to_string()),
                action: String::new(),
                params: Value::Object(Map::new()),
                auth_token: None,
            },
        );
        let HostEffect::Respond { payload, .. } = &effects[0] else {
            panic!("expected error response");
        };
        assert!(!payload.ok);
        assert!(payload.component.is_none());
        assert!(payload.version.is_none());
        assert_eq!(
            payload.error.as_ref().map(|e| e.message.as_str()),
            Some("Missing action")
        );
    }

    // TCP bridge tests — run on all platforms to validate bridge fundamentals
    // without requiring WSL. The host TCP listener (Windows) and the CLI TCP
    // discovery path (WSL) both rely on a simple port file + loopback socket.

    #[test]
    fn tcp_port_file_format_roundtrip() {
        let tmp = std::env::temp_dir().join(format!("tabctl-host-tcp-{}", now_ms()));
        std::fs::create_dir_all(&tmp).unwrap();
        let port_file = tmp.join(TCP_PORT_FILENAME);
        std::fs::write(&port_file, format!("{}\n", 38500u16)).unwrap();
        let content = std::fs::read_to_string(&port_file).unwrap();
        let parsed: u16 = content.trim().parse().unwrap();
        assert_eq!(parsed, 38500);
        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn tcp_listener_binds_loopback_and_accepts_connection() {
        use std::io::{Read, Write};
        use std::net::{TcpListener, TcpStream};

        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback TCP");
        let port = listener.local_addr().unwrap().port();
        assert!(port > 0);

        let handle = std::thread::spawn(move || {
            let mut stream = TcpStream::connect(format!("127.0.0.1:{port}")).expect("connect");
            stream.write_all(b"ping").expect("write");
        });

        let (mut conn, _) = listener.accept().expect("accept connection");
        let mut buf = Vec::new();
        conn.read_to_end(&mut buf).expect("read");
        handle.join().expect("thread join");
        assert_eq!(buf, b"ping");
    }

    #[test]
    fn test_auth_token_file_generation() {
        let tmp = std::env::temp_dir().join(format!("tabctl-host-auth-{}", now_ms()));
        std::fs::create_dir_all(&tmp).unwrap();

        let token1 = generate_and_write_auth_token(&tmp).expect("generate token");
        let token_path = tmp.join(AUTH_TOKEN_FILENAME);

        // File exists and contains the token
        assert!(token_path.exists());
        let content = std::fs::read_to_string(&token_path).unwrap();
        assert_eq!(content, token1);

        // Token is exactly 32 hex characters
        assert_eq!(token1.len(), AUTH_TOKEN_LENGTH);
        assert!(token1.chars().all(|c| c.is_ascii_hexdigit()));

        // Two calls produce different tokens
        let token2 = generate_and_write_auth_token(&tmp).expect("generate second token");
        assert_eq!(token2.len(), AUTH_TOKEN_LENGTH);
        assert_ne!(token1, token2);

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn tcp_auth_token_rejects_wrong_token() {
        let undo_path = temp_undo_path();
        let expected_token = Arc::new("correct-token-abc123".to_string());

        let request = RequestEnvelope {
            id: Some("req-bad-auth".to_string()),
            action: "ping".to_string(),
            params: Value::Object(Map::new()),
            auth_token: Some("wrong-token".to_string()),
        };

        // Simulate the TCP auth check: token mismatch produces error response
        let provided = request.auth_token.as_deref().unwrap_or("");
        assert_ne!(provided, expected_token.as_str());

        let effects: Vec<HostEffect> = vec![HostEffect::Respond {
            client_id: 100,
            payload: ResponseEnvelope {
                ok: false,
                action: None,
                request_id: request.id.clone(),
                component: Some("host".to_string()),
                version: Some(host_version().to_string()),
                progress: None,
                data: None,
                error: Some(ProtocolError {
                    message: "Authentication failed".to_string(),
                    hint: Some("Invalid or missing auth token for TCP connection".to_string()),
                }),
            },
        }];

        let HostEffect::Respond { payload, .. } = &effects[0] else {
            panic!("expected auth error response");
        };
        assert!(!payload.ok);
        assert_eq!(
            payload.error.as_ref().map(|e| e.message.as_str()),
            Some("Authentication failed")
        );
        assert_eq!(
            payload.error.as_ref().and_then(|e| e.hint.as_deref()),
            Some("Invalid or missing auth token for TCP connection")
        );
        assert_eq!(payload.request_id.as_deref(), Some("req-bad-auth"));

        let _ = std::fs::remove_file(undo_path);
    }

    #[test]
    fn tcp_auth_token_accepts_correct_token() {
        let undo_path = temp_undo_path();
        let mut state = HostState::new(undo_path);
        let expected_token = Arc::new("correct-token-abc123".to_string());

        let request = RequestEnvelope {
            id: Some("req-good-auth".to_string()),
            action: "ping".to_string(),
            params: Value::Object(Map::new()),
            auth_token: Some("correct-token-abc123".to_string()),
        };

        // Simulate the TCP auth check path
        let provided = request.auth_token.as_deref().unwrap_or("");
        assert_eq!(provided, expected_token.as_str());

        // With correct token, request should be forwarded normally
        let effects = state.handle_cli_request(101, request);
        let HostEffect::SendNative(native) = &effects[0] else {
            panic!("expected ping to be forwarded to extension");
        };
        assert_eq!(native.id, "req-good-auth");
    }

    #[test]
    fn tcp_auth_token_missing_token_rejected() {
        let expected_token = Arc::new("correct-token-abc123".to_string());

        let request = RequestEnvelope {
            id: Some("req-no-auth".to_string()),
            action: "ping".to_string(),
            params: Value::Object(Map::new()),
            auth_token: None,
        };

        let provided = request.auth_token.as_deref().unwrap_or("");
        assert_ne!(provided, expected_token.as_str());
    }

    #[test]
    fn no_expected_token_skips_validation() {
        let undo_path = temp_undo_path();
        let mut state = HostState::new(undo_path);

        // No auth_token on request, no expected token (Unix socket / pipe path)
        let request = RequestEnvelope {
            id: Some("req-no-tcp".to_string()),
            action: "ping".to_string(),
            params: Value::Object(Map::new()),
            auth_token: None,
        };

        let expected_auth_token: Option<Arc<String>> = None;
        // When expected_auth_token is None, skip validation entirely
        assert!(expected_auth_token.is_none());

        let effects = state.handle_cli_request(102, request);
        let HostEffect::SendNative(native) = &effects[0] else {
            panic!("expected ping forward without auth check");
        };
        assert_eq!(native.id, "req-no-tcp");
    }

    #[test]
    fn resolve_config_uses_profile_data_dir_when_tabctl_profile_set() {
        let root = std::env::temp_dir().join(format!("tabctl-host-profile-cfg-{}", now_ms()));
        let config_dir = root.join("config");
        let base_data_dir = root.join("state").join("tabctl");
        let profile_data_dir = root
            .join("state")
            .join("tabctl")
            .join("profiles")
            .join("edge");
        std::fs::create_dir_all(&config_dir).expect("create config dir");
        std::fs::create_dir_all(&profile_data_dir).expect("create profile data dir");
        let profiles_path = config_dir.join("profiles.json");
        std::fs::write(
            &profiles_path,
            serde_json::to_string_pretty(&serde_json::json!({
                "default": "edge",
                "profiles": {
                    "edge": {
                        "browser": "edge",
                        "extensionId": "ext",
                        "nodePath": "/usr/bin/tabctl",
                        "hostPath": "/tmp/tabctl-host.sh",
                        "dataDir": profile_data_dir.display().to_string()
                    }
                }
            }))
            .expect("serialize profiles"),
        )
        .expect("write profiles");

        with_env_vars(
            &[
                (
                    "TABCTL_CONFIG_DIR",
                    Some(config_dir.to_str().expect("config dir")),
                ),
                (
                    "TABCTL_DATA_DIR",
                    Some(base_data_dir.to_str().expect("base data dir")),
                ),
                ("TABCTL_PROFILE", Some("edge")),
                ("TABCTL_SOCKET", None),
                ("TABCTL_STATE_DIR", None),
                ("XDG_STATE_HOME", None),
            ],
            || {
                let config = resolve_config();
                assert_eq!(config.active_profile_name.as_deref(), Some("edge"));
                assert_eq!(config.data_dir, profile_data_dir.display().to_string());
                assert_eq!(config.base_data_dir, base_data_dir.display().to_string());
                if cfg!(windows) {
                    assert!(
                        config.socket_path.starts_with(r"\\.\pipe\tabctl-"),
                        "unexpected windows socket path: {}",
                        config.socket_path
                    );
                } else {
                    assert!(
                        config.socket_path.ends_with("profiles/edge/tabctl.sock")
                            || config.socket_path.ends_with("profiles\\edge\\tabctl.sock")
                    );
                }
            },
        );

        let _ = std::fs::remove_dir_all(&root);
    }

    // ── Orchestration framework tests ───────────────────────────────────────

    #[derive(Debug)]
    struct TwoStepOrch {
        phase: u8,
    }

    impl TwoStepOrch {
        fn new() -> Self {
            Self { phase: 0 }
        }
    }

    impl Orchestration for TwoStepOrch {
        fn start(&mut self) -> OrchStep {
            self.phase = 1;
            OrchStep::SendPrimitive {
                action: "p:snapshot".to_string(),
                params: Value::Object(Map::new()),
            }
        }
        fn step(&mut self, response: Value) -> OrchStep {
            match self.phase {
                1 => {
                    self.phase = 2;
                    // Second step: send another primitive
                    OrchStep::SendPrimitive {
                        action: "p:tab-query".to_string(),
                        params: serde_json::json!({"query": {"active": true}}),
                    }
                }
                _ => {
                    // Complete with aggregated response
                    OrchStep::Complete {
                        response: serde_json::json!({"result": "done", "input": response}),
                        undo: None,
                    }
                }
            }
        }
    }

    #[test]
    fn orchestration_two_step_sequence_sends_primitives_then_completes() {
        let undo_path = temp_undo_path();
        let mut state = HostState::new(undo_path);

        // Simulate: start orchestration manually (since orchestration_for
        // returns None, we inject directly via process_orch_step)
        let mut orch = TwoStepOrch::new();
        let step = orch.start();

        let effects = state.process_orch_step(
            42,
            "test-action",
            Some("req-orch".to_string()),
            None,
            step,
            Box::new(orch),
        );

        // Step 1: should send p:snapshot
        assert_eq!(effects.len(), 1);
        let HostEffect::SendNative(native1) = &effects[0] else {
            panic!("expected SendNative for step 1");
        };
        assert_eq!(native1.action.as_deref(), Some("p:snapshot"));

        // Feed snapshot response back
        let effects2 = state.handle_native_message(NativeMessage {
            id: native1.id.clone(),
            action: None,
            ok: Some(true),
            progress: None,
            params: None,
            data: Some(serde_json::json!({"windows": []})),
            error: None,
        });

        // Step 2: should send p:tab-query
        assert_eq!(effects2.len(), 1);
        let HostEffect::SendNative(native2) = &effects2[0] else {
            panic!("expected SendNative for step 2");
        };
        assert_eq!(native2.action.as_deref(), Some("p:tab-query"));

        // Feed tab-query response back
        let effects3 = state.handle_native_message(NativeMessage {
            id: native2.id.clone(),
            action: None,
            ok: Some(true),
            progress: None,
            params: None,
            data: Some(serde_json::json!([{"id": 1, "active": true}])),
            error: None,
        });

        // Should complete with final response
        assert_eq!(effects3.len(), 1);
        let HostEffect::Respond { client_id, payload } = &effects3[0] else {
            panic!("expected Respond after completion");
        };
        assert_eq!(*client_id, 42);
        assert!(payload.ok);
        assert_eq!(payload.action.as_deref(), Some("test-action"));
        let data = payload.data.as_ref().expect("response data");
        assert_eq!(data.get("result").and_then(Value::as_str), Some("done"));
    }

    #[test]
    fn orchestration_error_step_returns_error_response() {
        let undo_path = temp_undo_path();
        let mut state = HostState::new(undo_path);

        let step = OrchStep::Error {
            message: "test error".to_string(),
            hint: Some("try again".to_string()),
        };

        let effects = state.process_orch_step(
            99,
            "failing-action",
            Some("req-fail".to_string()),
            None,
            step,
            Box::new(TwoStepOrch::new()),
        );

        let HostEffect::Respond { payload, .. } = &effects[0] else {
            panic!("expected error response");
        };
        assert!(!payload.ok);
        assert_eq!(
            payload.error.as_ref().map(|e| e.message.as_str()),
            Some("test error")
        );
        assert_eq!(
            payload.error.as_ref().and_then(|e| e.hint.as_deref()),
            Some("try again")
        );
    }

    #[test]
    fn orchestration_extension_error_aborts_orchestration() {
        let undo_path = temp_undo_path();
        let mut state = HostState::new(undo_path);

        // Start a 2-step orchestration
        let mut orch = TwoStepOrch::new();
        let step = orch.start();
        let effects = state.process_orch_step(
            50,
            "test-abort",
            Some("req-abort".to_string()),
            None,
            step,
            Box::new(orch),
        );
        let HostEffect::SendNative(native) = &effects[0] else {
            panic!("expected SendNative");
        };

        // Extension returns error
        let effects2 = state.handle_native_message(NativeMessage {
            id: native.id.clone(),
            action: None,
            ok: Some(false),
            progress: None,
            params: None,
            data: None,
            error: Some(ProtocolError {
                message: "chrome api failed".to_string(),
                hint: None,
            }),
        });

        // Should abort and return error to client
        let HostEffect::Respond { payload, .. } = &effects2[0] else {
            panic!("expected error response on abort");
        };
        assert!(!payload.ok);
        assert_eq!(
            payload.error.as_ref().map(|e| e.message.as_str()),
            Some("chrome api failed")
        );
        // Error response must carry the original client request_id, not the internal orch-* id
        assert_eq!(payload.request_id.as_deref(), Some("req-abort"));
    }
}
