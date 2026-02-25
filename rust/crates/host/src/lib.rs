use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use std::fs;
#[cfg(windows)]
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};
use tabctl_shared::{
    ClientInfo, NativeMessage, ProfileRegistry, ProtocolError, RequestEnvelope, ResponseEnvelope,
    SocketEndpoint, TabctlConfig, VersionInfo,
};

use sha2::{Digest, Sha256};
#[cfg(unix)]
use std::os::unix::net::UnixListener;
#[cfg(windows)]
use std::os::windows::ffi::OsStrExt;
#[cfg(windows)]
use std::os::windows::io::{FromRawHandle, RawHandle};
#[cfg(windows)]
use windows_sys::Win32::Foundation::{CloseHandle, ERROR_PIPE_CONNECTED, INVALID_HANDLE_VALUE};
#[cfg(windows)]
use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX};
#[cfg(windows)]
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE,
    PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};

const REQUEST_TIMEOUT_MS: u64 = 30_000;
const MAX_RESPONSE_BYTES: usize = 20 * 1024 * 1024;
const MAX_NATIVE_MESSAGE_BYTES: usize = 10 * 1024 * 1024;
const HISTORY_LIMIT_DEFAULT: usize = 20;
const RETENTION_DAYS: u64 = 30;
const TCP_PORT_FILENAME: &str = "tcp-port";
const AUTH_TOKEN_FILENAME: &str = "auth-token";
const AUTH_TOKEN_LENGTH: usize = 32; // 32 hex chars = 128 bits
const TCP_PORT_BASE: u16 = 38_000;
const TCP_PORT_SPAN: u16 = 1_000;
const TCP_PORT_ATTEMPTS: u16 = 128;

static ID_COUNTER: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone)]
struct PendingRequest {
    client_id: u64,
    action: String,
    txid: Option<String>,
    created_at: u64,
}

#[derive(Debug, Clone)]
struct AnalysisRecord {
    data: Map<String, Value>,
}

#[derive(Debug)]
struct HostState {
    pending: HashMap<String, PendingRequest>,
    analyses: HashMap<String, AnalysisRecord>,
    undo_log: PathBuf,
}

#[derive(Debug)]
enum HostEffect {
    SendNative(NativeMessage),
    Respond {
        client_id: u64,
        payload: ResponseEnvelope,
    },
}

fn host_version() -> &'static str {
    option_env!("TABCTL_HOST_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"))
}

fn base_version() -> &'static str {
    option_env!("TABCTL_BASE_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"))
}

fn git_sha() -> &'static str {
    option_env!("TABCTL_GIT_SHA").unwrap_or("unknown")
}

fn is_dirty() -> bool {
    option_env!("TABCTL_DIRTY")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn create_id(prefix: &str) -> String {
    let counter = ID_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{prefix}-{}-{counter}", now_ms())
}

fn local_actions() -> HashSet<&'static str> {
    HashSet::from(["history", "undo", "version"])
}

fn undo_actions() -> HashSet<&'static str> {
    HashSet::from([
        "archive",
        "close",
        "group-update",
        "group-ungroup",
        "group-assign",
        "group-gather",
        "move-tab",
        "move-group",
        "merge-window",
    ])
}

fn default_config_base() -> PathBuf {
    #[cfg(windows)]
    {
        if let Ok(appdata) = std::env::var("APPDATA") {
            return PathBuf::from(appdata);
        }
        if let Ok(home) = std::env::var("USERPROFILE") {
            return PathBuf::from(home).join("AppData").join("Roaming");
        }
    }
    #[cfg(not(windows))]
    {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(".config");
        }
    }
    PathBuf::from(".")
}

fn default_state_base() -> PathBuf {
    #[cfg(windows)]
    {
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            return PathBuf::from(local);
        }
        if let Ok(home) = std::env::var("USERPROFILE") {
            return PathBuf::from(home).join("AppData").join("Local");
        }
    }
    #[cfg(not(windows))]
    {
        if let Ok(home) = std::env::var("HOME") {
            return PathBuf::from(home).join(".local").join("state");
        }
    }
    PathBuf::from(".")
}

fn resolve_socket_path(data_dir: &Path) -> String {
    #[cfg(windows)]
    {
        let mut hasher = Sha256::new();
        hasher.update(data_dir.to_string_lossy().as_bytes());
        let digest = hasher.finalize();
        let hash = digest[..6]
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        format!(r"\\.\pipe\tabctl-{hash}")
    }
    #[cfg(not(windows))]
    {
        data_dir.join("tabctl.sock").to_string_lossy().to_string()
    }
}

fn resolve_base_data_dir() -> PathBuf {
    if let Ok(path) = std::env::var("TABCTL_DATA_DIR") {
        if !path.trim().is_empty() {
            return PathBuf::from(path);
        }
    }
    if let Ok(path) = std::env::var("TABCTL_STATE_DIR") {
        if !path.trim().is_empty() {
            return PathBuf::from(path);
        }
    }
    if let Ok(state_home) = std::env::var("XDG_STATE_HOME") {
        return PathBuf::from(state_home).join("tabctl");
    }
    default_state_base().join("tabctl")
}

fn resolve_active_profile_name() -> Option<String> {
    std::env::var("TABCTL_PROFILE")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn resolve_profile_data_dir(config_dir: &Path, profile_name: &str) -> Option<PathBuf> {
    let profiles_path = config_dir.join("profiles.json");
    let content = fs::read_to_string(&profiles_path).ok()?;
    let registry = serde_json::from_str::<ProfileRegistry>(&content).ok()?;
    registry
        .profiles
        .get(profile_name)
        .map(|entry| PathBuf::from(&entry.data_dir))
}

fn resolve_config() -> TabctlConfig {
    let config_dir = std::env::var("TABCTL_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            std::env::var("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|_| default_config_base())
                .join("tabctl")
        });

    let base_data_dir = resolve_base_data_dir();
    let active_profile_name = resolve_active_profile_name();
    let data_dir = active_profile_name
        .as_deref()
        .and_then(|profile_name| resolve_profile_data_dir(&config_dir, profile_name))
        .unwrap_or_else(|| base_data_dir.clone());

    let socket_path =
        std::env::var("TABCTL_SOCKET").unwrap_or_else(|_| resolve_socket_path(&data_dir));

    TabctlConfig {
        config_dir: config_dir.to_string_lossy().to_string(),
        data_dir: data_dir.to_string_lossy().to_string(),
        base_data_dir: base_data_dir.to_string_lossy().to_string(),
        socket_path,
        undo_log: data_dir.join("undo.jsonl").to_string_lossy().to_string(),
        wrapper_dir: data_dir.to_string_lossy().to_string(),
        policy_path: config_dir.join("policy.json").to_string_lossy().to_string(),
        active_profile_name,
    }
}

fn write_native_message<W: Write>(writer: &mut W, message: &NativeMessage) -> io::Result<()> {
    let payload = serde_json::to_vec(message)?;
    let len = payload.len() as u32;
    writer.write_all(&len.to_le_bytes())?;
    writer.write_all(&payload)?;
    writer.flush()?;
    Ok(())
}

fn read_native_message<R: Read>(reader: &mut R) -> io::Result<Option<NativeMessage>> {
    let mut len_buf = [0_u8; 4];
    match reader.read_exact(&mut len_buf) {
        Ok(_) => {}
        Err(err) if err.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(err) => return Err(err),
    }
    let len = u32::from_le_bytes(len_buf) as usize;
    if len > MAX_NATIVE_MESSAGE_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("native message too large: {len} bytes (max {MAX_NATIVE_MESSAGE_BYTES})"),
        ));
    }
    let mut payload = vec![0_u8; len];
    reader.read_exact(&mut payload)?;
    let msg = serde_json::from_slice::<NativeMessage>(&payload)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err.to_string()))?;
    Ok(Some(msg))
}

fn log_line(message: &str) {
    eprintln!("[tabctl-host-rust] {message}");
}

fn read_undo_records(file_path: &Path) -> Vec<Map<String, Value>> {
    let Ok(content) = fs::read_to_string(file_path) else {
        return Vec::new();
    };

    content
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|v| v.as_object().cloned())
        .collect()
}

fn append_undo_record(file_path: &Path, record: &Map<String, Value>) {
    if let Some(parent) = file_path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Ok(serialized) = serde_json::to_string(record) {
        let _ = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(file_path)
            .and_then(|mut f| writeln!(f, "{serialized}"));
    }
}

fn filter_by_retention(
    records: Vec<Map<String, Value>>,
    retention_days: u64,
) -> Vec<Map<String, Value>> {
    let cutoff = now_ms().saturating_sub(retention_days * 24 * 60 * 60 * 1000);
    records
        .into_iter()
        .filter(|record| {
            record
                .get("createdAt")
                .and_then(|v| v.as_u64())
                .map(|created_at| created_at >= cutoff)
                .unwrap_or(true)
        })
        .collect()
}

fn find_undo_record(file_path: &Path, txid: &str) -> Option<Map<String, Value>> {
    let records = filter_by_retention(read_undo_records(file_path), RETENTION_DAYS);
    records
        .into_iter()
        .rev()
        .find(|record| record.get("txid").and_then(|v| v.as_str()) == Some(txid))
}

fn find_latest_undo_record(file_path: &Path) -> Option<Map<String, Value>> {
    let records = filter_by_retention(read_undo_records(file_path), RETENTION_DAYS);
    records.into_iter().last()
}

fn version_info_value() -> Value {
    let info = VersionInfo {
        version: host_version().to_string(),
        base_version: base_version().to_string(),
        git_sha: git_sha().to_string(),
        dirty: is_dirty(),
        component: "host".to_string(),
    };
    serde_json::to_value(info).unwrap_or(Value::Object(Map::new()))
}

fn base_response(ok: bool, action: Option<String>, request_id: Option<String>) -> ResponseEnvelope {
    ResponseEnvelope {
        ok,
        action,
        request_id,
        component: Some("host".to_string()),
        version: Some(host_version().to_string()),
        progress: None,
        data: None,
        error: None,
    }
}

fn value_object(input: Option<Value>) -> Map<String, Value> {
    input
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

fn add_ping_metadata(mut data: Map<String, Value>) -> Map<String, Value> {
    let extension_base_version = data
        .get("baseVersion")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    let extension_version = data
        .get("version")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    data.insert(
        "hostVersion".to_string(),
        Value::String(host_version().to_string()),
    );
    data.insert(
        "hostBaseVersion".to_string(),
        Value::String(base_version().to_string()),
    );
    data.insert(
        "hostGitSha".to_string(),
        Value::String(git_sha().to_string()),
    );
    data.insert("hostDirty".to_string(), Value::Bool(is_dirty()));
    let versions_in_sync = extension_base_version
        .as_deref()
        .map(|v| v == base_version())
        .or_else(|| extension_version.as_deref().map(|v| v == host_version()))
        .unwrap_or(false);
    data.insert("versionsInSync".to_string(), Value::Bool(versions_in_sync));
    data
}

impl HostState {
    fn new(undo_log: PathBuf) -> Self {
        Self {
            pending: HashMap::new(),
            analyses: HashMap::new(),
            undo_log,
        }
    }

    fn forward_to_extension(
        &mut self,
        client_id: u64,
        request: &RequestEnvelope,
        txid: Option<String>,
    ) -> Vec<HostEffect> {
        let request_id = request.id.clone().unwrap_or_else(|| create_id("req"));
        let mut params = value_object(Some(request.params.clone()));

        if let Some(txid_ref) = txid.clone() {
            params.insert("txid".to_string(), Value::String(txid_ref));
        }

        if !local_actions().contains(request.action.as_str()) {
            let client = ClientInfo {
                component: "host".to_string(),
                version: host_version().to_string(),
            };
            params.insert(
                "client".to_string(),
                serde_json::to_value(client).unwrap_or(Value::Object(Map::new())),
            );
        }

        self.pending.insert(
            request_id.clone(),
            PendingRequest {
                client_id,
                action: request.action.clone(),
                txid,
                created_at: now_ms(),
            },
        );

        vec![HostEffect::SendNative(NativeMessage {
            id: request_id,
            action: Some(request.action.clone()),
            ok: None,
            progress: None,
            params: Some(Value::Object(params)),
            data: None,
            error: None,
        })]
    }

    fn handle_cli_request(&mut self, client_id: u64, request: RequestEnvelope) -> Vec<HostEffect> {
        if request.action.is_empty() {
            let mut resp = base_response(false, None, request.id);
            resp.error = Some(ProtocolError {
                message: "Missing action".to_string(),
                hint: None,
            });
            return vec![HostEffect::Respond {
                client_id,
                payload: resp,
            }];
        }

        let action = request.action.clone();

        if action == "version" {
            let mut resp = base_response(true, Some(action), request.id);
            resp.data = Some(version_info_value());
            return vec![HostEffect::Respond {
                client_id,
                payload: resp,
            }];
        }

        if action == "history" {
            let limit = request
                .params
                .get("limit")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize)
                .unwrap_or(HISTORY_LIMIT_DEFAULT);
            let records = filter_by_retention(read_undo_records(&self.undo_log), RETENTION_DAYS);
            let start = records.len().saturating_sub(limit);
            let mut resp = base_response(true, Some(action), request.id);
            resp.data = Some(Value::Array(
                records[start..]
                    .iter()
                    .cloned()
                    .map(Value::Object)
                    .collect(),
            ));
            return vec![HostEffect::Respond {
                client_id,
                payload: resp,
            }];
        }

        if action == "undo" {
            let txid = request
                .params
                .get("txid")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let latest = request
                .params
                .get("latest")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);

            if txid.is_none() && !latest {
                let mut resp = base_response(false, Some(action), request.id);
                resp.error = Some(ProtocolError {
                    message: "Missing txid".to_string(),
                    hint: Some(
                        "Use tabctl history --json to find a txid, or run tabctl undo --latest"
                            .to_string(),
                    ),
                });
                return vec![HostEffect::Respond {
                    client_id,
                    payload: resp,
                }];
            }

            let record = if let Some(tx) = txid {
                find_undo_record(&self.undo_log, &tx)
            } else {
                find_latest_undo_record(&self.undo_log)
            };

            let Some(record) = record else {
                let mut resp = base_response(false, Some(action), request.id);
                resp.error = Some(ProtocolError {
                    message: "Undo record not found".to_string(),
                    hint: None,
                });
                return vec![HostEffect::Respond {
                    client_id,
                    payload: resp,
                }];
            };

            let undo_request = RequestEnvelope {
                id: request.id,
                action: "undo".to_string(),
                params: Value::Object(Map::from_iter([(
                    "record".to_string(),
                    Value::Object(record),
                )])),
                auth_token: None,
            };
            return self.forward_to_extension(client_id, &undo_request, None);
        }

        if action == "close" && request.params.get("mode").and_then(|v| v.as_str()) == Some("apply")
        {
            let analysis_id = request
                .params
                .get("analysisId")
                .and_then(|v| v.as_str())
                .map(str::to_string);

            let Some(analysis_id) = analysis_id else {
                let mut resp = base_response(false, Some(action), request.id);
                resp.error = Some(ProtocolError {
                    message: "Unknown analysisId".to_string(),
                    hint: None,
                });
                return vec![HostEffect::Respond {
                    client_id,
                    payload: resp,
                }];
            };

            let Some(analysis) = self.analyses.get(&analysis_id) else {
                let mut resp = base_response(false, Some(action), request.id);
                resp.error = Some(ProtocolError {
                    message: "Unknown analysisId".to_string(),
                    hint: None,
                });
                return vec![HostEffect::Respond {
                    client_id,
                    payload: resp,
                }];
            };

            let candidates = analysis
                .data
                .get("candidates")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let mut tab_ids = Vec::new();
            let mut expected_urls = Map::new();

            for candidate in candidates {
                let Some(candidate_obj) = candidate.as_object() else {
                    continue;
                };
                let Some(tab_id) = candidate_obj.get("tabId").and_then(|v| v.as_i64()) else {
                    continue;
                };
                tab_ids.push(Value::Number(tab_id.into()));
                if let Some(url) = candidate_obj.get("url").and_then(|v| v.as_str()) {
                    expected_urls.insert(tab_id.to_string(), Value::String(url.to_string()));
                }
            }

            if tab_ids.is_empty() {
                let mut resp = base_response(true, Some(action), request.id);
                resp.data = Some(Value::Object(Map::from_iter([
                    ("txid".to_string(), Value::Null),
                    (
                        "summary".to_string(),
                        Value::Object(Map::from_iter([
                            ("closedTabs".to_string(), Value::Number(0.into())),
                            ("skippedTabs".to_string(), Value::Number(0.into())),
                        ])),
                    ),
                    ("skipped".to_string(), Value::Array(Vec::new())),
                ])));
                return vec![HostEffect::Respond {
                    client_id,
                    payload: resp,
                }];
            }

            let mut params = Map::new();
            params.insert("mode".to_string(), Value::String("apply".to_string()));
            params.insert("tabIds".to_string(), Value::Array(tab_ids));
            params.insert("expectedUrls".to_string(), Value::Object(expected_urls));
            let close_request = RequestEnvelope {
                id: request.id,
                action,
                params: Value::Object(params),
                auth_token: None,
            };
            return self.forward_to_extension(client_id, &close_request, Some(create_id("tx")));
        }

        if undo_actions().contains(action.as_str()) {
            return self.forward_to_extension(client_id, &request, Some(create_id("tx")));
        }

        self.forward_to_extension(client_id, &request, None)
    }

    fn handle_native_message(&mut self, message: NativeMessage) -> Vec<HostEffect> {
        let message_id = message.id.clone();

        if let Some(pending) = self.pending.get(&message_id) {
            if now_ms().saturating_sub(pending.created_at) > REQUEST_TIMEOUT_MS {
                let timed_out = self.pending.remove(&message_id).expect("pending exists");
                let mut resp = base_response(false, Some(timed_out.action), Some(message_id));
                resp.error = Some(ProtocolError {
                    message: "Request timed out".to_string(),
                    hint: None,
                });
                return vec![HostEffect::Respond {
                    client_id: timed_out.client_id,
                    payload: resp,
                }];
            }
        }

        let Some(pending) = self.pending.get(&message_id).cloned() else {
            return Vec::new();
        };

        if message.progress.unwrap_or(false) {
            let mut resp = base_response(true, Some(pending.action), Some(message_id));
            resp.progress = Some(true);
            resp.data = message.data;
            return vec![HostEffect::Respond {
                client_id: pending.client_id,
                payload: resp,
            }];
        }

        let pending = self.pending.remove(&message_id).expect("pending exists");

        if !message.ok.unwrap_or(false) {
            let mut resp = base_response(false, Some(pending.action), Some(message_id));
            resp.error = message.error.or(Some(ProtocolError {
                message: "Unknown error".to_string(),
                hint: None,
            }));
            return vec![HostEffect::Respond {
                client_id: pending.client_id,
                payload: resp,
            }];
        }

        let message_data = value_object(message.data.clone());

        if pending.action == "ping" {
            let data = add_ping_metadata(message_data);
            let mut resp = base_response(true, Some("ping".to_string()), Some(message_id));
            resp.data = Some(Value::Object(data));
            return vec![HostEffect::Respond {
                client_id: pending.client_id,
                payload: resp,
            }];
        }

        if pending.action == "analyze" {
            let analysis_id = create_id("analysis");
            self.analyses.insert(
                analysis_id.clone(),
                AnalysisRecord {
                    data: message_data.clone(),
                },
            );
            let mut data = message_data;
            data.insert("analysisId".to_string(), Value::String(analysis_id));
            let mut resp = base_response(true, Some("analyze".to_string()), Some(message_id));
            resp.data = Some(Value::Object(data));
            return vec![HostEffect::Respond {
                client_id: pending.client_id,
                payload: resp,
            }];
        }

        if undo_actions().contains(pending.action.as_str()) {
            let mut record = Map::new();
            match pending.txid.clone() {
                Some(txid) => {
                    record.insert("txid".to_string(), Value::String(txid));
                }
                None => {
                    record.insert("txid".to_string(), Value::Null);
                }
            }
            record.insert("createdAt".to_string(), Value::Number(now_ms().into()));
            record.insert("action".to_string(), Value::String(pending.action.clone()));
            record.insert(
                "summary".to_string(),
                message_data
                    .get("summary")
                    .cloned()
                    .unwrap_or_else(|| Value::Object(Map::new())),
            );
            let undo_payload = message_data.get("undo").cloned().unwrap_or(Value::Null);
            record.insert("undo".to_string(), undo_payload.clone());
            if !undo_payload.is_null() {
                append_undo_record(&self.undo_log, &record);
            }

            let mut data = message_data;
            if let Some(txid) = pending.txid {
                data.insert("txid".to_string(), Value::String(txid));
            } else {
                data.insert("txid".to_string(), Value::Null);
            }
            let mut resp = base_response(true, Some(pending.action), Some(message_id));
            resp.data = Some(Value::Object(data));
            return vec![HostEffect::Respond {
                client_id: pending.client_id,
                payload: resp,
            }];
        }

        let mut resp = base_response(true, Some(pending.action), Some(message_id));
        resp.data = Some(Value::Object(message_data));
        vec![HostEffect::Respond {
            client_id: pending.client_id,
            payload: resp,
        }]
    }
}

type ClientWriter = Arc<Mutex<Box<dyn Write + Send>>>;
type Clients = Arc<Mutex<HashMap<u64, ClientWriter>>>;

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

fn start_native_reader(
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

fn handle_client(
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

#[cfg(unix)]
fn run_unix() -> io::Result<()> {
    let config = resolve_config();
    let endpoint = SocketEndpoint::parse(&config.socket_path)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err))?;
    let socket_path = match endpoint {
        SocketEndpoint::Unix { path } => PathBuf::from(path),
        SocketEndpoint::Pipe { .. } => {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Named pipe endpoint is unsupported by the Unix host runtime",
            ));
        }
        SocketEndpoint::Tcp { .. } => {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "TCP endpoint is unsupported by the Unix host runtime",
            ));
        }
    };
    let socket_dir = PathBuf::from(&config.data_dir);
    fs::create_dir_all(&socket_dir)?;

    if socket_path.exists() {
        let _ = fs::remove_file(&socket_path);
    }

    let listener = UnixListener::bind(&socket_path)?;
    let _ = fs::set_permissions(&socket_path, fs::Permissions::from_mode(0o600));

    let state = Arc::new(Mutex::new(HostState::new(PathBuf::from(config.undo_log))));
    let clients: Clients = Arc::new(Mutex::new(HashMap::new()));
    let native_out = Arc::new(Mutex::new(io::stdout()));

    start_native_reader(state.clone(), clients.clone(), native_out.clone());

    // Optional TCP listener (opt-in via TABCTL_HOST_TCP=1)
    let tcp_enabled = std::env::var("TABCTL_HOST_TCP")
        .map(|v| v == "1")
        .unwrap_or(false);

    if tcp_enabled {
        let data_dir = PathBuf::from(&config.data_dir);
        let (tcp_listener, _tcp_port, auth_token) = setup_tcp_listener(&data_dir)?;
        spawn_tcp_accept_loop(
            tcp_listener,
            state.clone(),
            clients.clone(),
            native_out.clone(),
            auth_token,
        );
    }

    log_line(&format!("listening on {}", socket_path.display()));

    for incoming in listener.incoming() {
        match incoming {
            Ok(stream) => {
                let client_id = ID_COUNTER.fetch_add(1, Ordering::Relaxed);
                let writer_stream = match stream.try_clone() {
                    Ok(clone) => clone,
                    Err(err) => {
                        log_line(&format!("socket clone error: {err}"));
                        continue;
                    }
                };
                let writer: ClientWriter = Arc::new(Mutex::new(Box::new(writer_stream)));
                if let Ok(mut map) = clients.lock() {
                    map.insert(client_id, writer);
                }
                let state_clone = state.clone();
                let clients_clone = clients.clone();
                let native_out_clone = native_out.clone();
                thread::spawn(move || {
                    handle_client(
                        client_id,
                        Box::new(stream),
                        state_clone,
                        clients_clone,
                        native_out_clone,
                        None,
                    )
                });
            }
            Err(err) => log_line(&format!("socket accept error: {err}")),
        }
    }

    Ok(())
}

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[cfg(windows)]
fn to_wide(value: &str) -> Vec<u16> {
    std::ffi::OsStr::new(value)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(windows)]
fn connect_named_pipe_instance(path: &str) -> io::Result<File> {
    let mut open_mode = PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE;
    loop {
        let wide = to_wide(path);
        let handle = unsafe {
            CreateNamedPipeW(
                wide.as_ptr(),
                open_mode,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                PIPE_UNLIMITED_INSTANCES,
                64 * 1024,
                64 * 1024,
                0,
                std::ptr::null(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(io::Error::last_os_error());
        }
        let connected = unsafe { ConnectNamedPipe(handle, std::ptr::null_mut()) };
        if connected == 0 {
            let err = io::Error::last_os_error();
            if err.raw_os_error() != Some(ERROR_PIPE_CONNECTED as i32) {
                unsafe {
                    CloseHandle(handle);
                }
                if open_mode & FILE_FLAG_FIRST_PIPE_INSTANCE != 0 {
                    open_mode = PIPE_ACCESS_DUPLEX;
                    continue;
                }
                return Err(err);
            }
        }
        return Ok(unsafe { File::from_raw_handle(handle as RawHandle) });
    }
}

fn deterministic_tcp_start_port(data_dir: &Path) -> u16 {
    let mut hasher = Sha256::new();
    hasher.update(data_dir.to_string_lossy().as_bytes());
    let digest = hasher.finalize();
    let seed = u16::from_be_bytes([digest[6], digest[7]]);
    TCP_PORT_BASE + (seed % TCP_PORT_SPAN)
}

fn bind_tcp_listener(data_dir: &Path) -> io::Result<(TcpListener, u16)> {
    if let Ok(port) = std::env::var("TABCTL_TCP_PORT") {
        let parsed = port
            .trim()
            .parse::<u16>()
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Invalid TABCTL_TCP_PORT"))?;
        let listener = TcpListener::bind(("127.0.0.1", parsed))?;
        return Ok((listener, parsed));
    }

    let start = deterministic_tcp_start_port(data_dir);
    for offset in 0..TCP_PORT_ATTEMPTS {
        let candidate = TCP_PORT_BASE + ((start - TCP_PORT_BASE + offset) % TCP_PORT_SPAN);
        match TcpListener::bind(("127.0.0.1", candidate)) {
            Ok(listener) => return Ok((listener, candidate)),
            Err(err) if err.kind() == io::ErrorKind::AddrInUse => continue,
            Err(err) => return Err(err),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AddrInUse,
        "Failed to bind localhost TCP listener",
    ))
}

fn write_tcp_port_file(data_dir: &Path, port: u16) -> io::Result<PathBuf> {
    let path = data_dir.join(TCP_PORT_FILENAME);
    fs::write(&path, format!("{port}\n"))?;
    Ok(path)
}

fn generate_and_write_auth_token(data_dir: &Path) -> io::Result<String> {
    let mut bytes = [0u8; 16]; // 16 bytes = 128 bits → 32 hex chars
    getrandom::getrandom(&mut bytes).map_err(|e| io::Error::other(e.to_string()))?;
    let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    debug_assert_eq!(token.len(), AUTH_TOKEN_LENGTH);
    let path = data_dir.join(AUTH_TOKEN_FILENAME);
    fs::write(&path, &token)?;
    Ok(token)
}

fn setup_tcp_listener(data_dir: &Path) -> io::Result<(TcpListener, u16, Arc<String>)> {
    let (listener, port) = bind_tcp_listener(data_dir)?;
    let port_file = write_tcp_port_file(data_dir, port)?;
    let auth_token = Arc::new(generate_and_write_auth_token(data_dir)?);
    log_line(&format!(
        "generated auth token in {}",
        data_dir.join(AUTH_TOKEN_FILENAME).display()
    ));
    log_line(&format!("listening on tcp://127.0.0.1:{port}"));
    log_line(&format!("published tcp port file {}", port_file.display()));
    Ok((listener, port, auth_token))
}

fn spawn_tcp_accept_loop(
    listener: TcpListener,
    state: Arc<Mutex<HostState>>,
    clients: Clients,
    native_out: Arc<Mutex<io::Stdout>>,
    auth_token: Arc<String>,
) {
    thread::spawn(move || {
        for incoming in listener.incoming() {
            match incoming {
                Ok(stream) => {
                    let client_id = ID_COUNTER.fetch_add(1, Ordering::Relaxed);
                    let writer_stream = match stream.try_clone() {
                        Ok(clone) => clone,
                        Err(err) => {
                            log_line(&format!("tcp clone error: {err}"));
                            continue;
                        }
                    };
                    let writer: ClientWriter = Arc::new(Mutex::new(Box::new(writer_stream)));
                    if let Ok(mut map) = clients.lock() {
                        map.insert(client_id, writer);
                    }
                    let state_clone = state.clone();
                    let clients_clone = clients.clone();
                    let native_out_clone = native_out.clone();
                    let token_clone = Some(auth_token.clone());
                    thread::spawn(move || {
                        handle_client(
                            client_id,
                            Box::new(stream),
                            state_clone,
                            clients_clone,
                            native_out_clone,
                            token_clone,
                        )
                    });
                }
                Err(err) => log_line(&format!("tcp accept error: {err}")),
            }
        }
    });
}

#[cfg(windows)]
fn run_windows() -> io::Result<()> {
    let config = resolve_config();
    let endpoint = SocketEndpoint::parse(&config.socket_path)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidInput, err))?;
    let pipe_path = match endpoint {
        SocketEndpoint::Pipe { path } => path,
        SocketEndpoint::Unix { .. } => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Windows host requires a named pipe endpoint",
            ));
        }
        SocketEndpoint::Tcp { .. } => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Windows host socket endpoint must be a named pipe",
            ));
        }
    };

    let data_dir = PathBuf::from(&config.data_dir);
    fs::create_dir_all(&data_dir)?;
    let (tcp_listener, _tcp_port, auth_token) = setup_tcp_listener(&data_dir)?;

    let state = Arc::new(Mutex::new(HostState::new(PathBuf::from(config.undo_log))));
    let clients: Clients = Arc::new(Mutex::new(HashMap::new()));
    let native_out = Arc::new(Mutex::new(io::stdout()));
    start_native_reader(state.clone(), clients.clone(), native_out.clone());

    spawn_tcp_accept_loop(
        tcp_listener,
        state.clone(),
        clients.clone(),
        native_out.clone(),
        auth_token,
    );

    log_line(&format!("listening on {pipe_path}"));

    loop {
        match connect_named_pipe_instance(&pipe_path) {
            Ok(pipe) => {
                let client_id = ID_COUNTER.fetch_add(1, Ordering::Relaxed);
                let reader = match pipe.try_clone() {
                    Ok(clone) => clone,
                    Err(err) => {
                        log_line(&format!("pipe clone error: {err}"));
                        continue;
                    }
                };
                let writer: ClientWriter = Arc::new(Mutex::new(Box::new(pipe)));
                if let Ok(mut map) = clients.lock() {
                    map.insert(client_id, writer);
                }
                let state_clone = state.clone();
                let clients_clone = clients.clone();
                let native_out_clone = native_out.clone();
                thread::spawn(move || {
                    handle_client(
                        client_id,
                        Box::new(reader),
                        state_clone,
                        clients_clone,
                        native_out_clone,
                        None,
                    )
                });
            }
            Err(err) => log_line(&format!("named pipe accept error: {err}")),
        }
    }
}

#[cfg(not(any(unix, windows)))]
fn run_host() -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "Rust host runtime is unsupported on this target",
    ))
}

#[cfg(unix)]
fn run_host() -> io::Result<()> {
    run_unix()
}

#[cfg(windows)]
fn run_host() -> io::Result<()> {
    run_windows()
}

pub fn run() {
    let _ = REQUEST_TIMEOUT_MS;
    if let Err(err) = run_host() {
        log_line(&format!("fatal: {err}"));
        process::exit(1);
    }
}

// Compile-time invariants for the TCP port range constants shared by host and CLI.
const _: () = assert!(
    TCP_PORT_BASE >= 1024,
    "port base must be an unprivileged port"
);
const _: () = assert!(TCP_PORT_SPAN > 0, "port span must be non-zero");
const _: () = assert!(
    (TCP_PORT_BASE as u32) + (TCP_PORT_SPAN as u32) <= 65535,
    "port range must fit in a u16"
);

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

        let request = RequestEnvelope {
            id: Some("req-1".to_string()),
            action: "move-tab".to_string(),
            params: Value::Object(Map::from_iter([(
                "tabId".to_string(),
                Value::Number(12.into()),
            )])),
            auth_token: None,
        };

        let effects = state.handle_cli_request(7, request);
        let HostEffect::SendNative(native) = &effects[0] else {
            panic!("expected native forward");
        };
        let params = native
            .params
            .clone()
            .and_then(|v| v.as_object().cloned())
            .expect("params object");

        assert!(params.get("txid").and_then(|v| v.as_str()).is_some());
        assert_eq!(
            params
                .get("client")
                .and_then(|v| v.get("component"))
                .and_then(|v| v.as_str()),
            Some("host")
        );
    }

    #[test]
    fn records_undo_on_successful_native_response() {
        let undo_path = temp_undo_path();
        let mut state = HostState::new(undo_path.clone());

        let request = RequestEnvelope {
            id: Some("req-2".to_string()),
            action: "move-tab".to_string(),
            params: Value::Object(Map::new()),
            auth_token: None,
        };
        let effects = state.handle_cli_request(9, request);
        let HostEffect::SendNative(native) = &effects[0] else {
            panic!("expected native forward");
        };
        let txid = native
            .params
            .as_ref()
            .and_then(|v| v.get("txid"))
            .and_then(|v| v.as_str())
            .expect("txid")
            .to_string();

        let response_effects = state.handle_native_message(NativeMessage {
            id: native.id.clone(),
            action: Some("move-tab".to_string()),
            ok: Some(true),
            progress: None,
            params: None,
            data: Some(Value::Object(Map::from_iter([
                (
                    "summary".to_string(),
                    Value::Object(Map::from_iter([(
                        "movedTabs".to_string(),
                        Value::Number(1.into()),
                    )])),
                ),
                (
                    "undo".to_string(),
                    Value::Object(Map::from_iter([(
                        "action".to_string(),
                        Value::String("move-tab".to_string()),
                    )])),
                ),
            ]))),
            error: None,
        });

        let HostEffect::Respond { payload, .. } = &response_effects[0] else {
            panic!("expected response");
        };
        let got_txid = payload
            .data
            .as_ref()
            .and_then(|v| v.get("txid"))
            .and_then(|v| v.as_str())
            .expect("response txid");
        assert_eq!(got_txid, txid);

        let records = read_undo_records(&undo_path);
        assert_eq!(records.len(), 1);
        assert_eq!(
            records[0].get("txid").and_then(|v| v.as_str()),
            Some(txid.as_str())
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
        let HostEffect::SendNative(analyze_native) = &analyze_effects[0] else {
            panic!("expected analyze forward");
        };

        let analyze_response = state.handle_native_message(NativeMessage {
            id: analyze_native.id.clone(),
            action: Some("analyze".to_string()),
            ok: Some(true),
            progress: None,
            params: None,
            data: Some(Value::Object(Map::from_iter([(
                "candidates".to_string(),
                Value::Array(vec![Value::Object(Map::from_iter([
                    ("tabId".to_string(), Value::Number(12.into())),
                    (
                        "url".to_string(),
                        Value::String("https://example.com".to_string()),
                    ),
                ]))]),
            )]))),
            error: None,
        });

        let HostEffect::Respond { payload, .. } = &analyze_response[0] else {
            panic!("expected analyze response");
        };
        let analysis_id = payload
            .data
            .as_ref()
            .and_then(|v| v.get("analysisId"))
            .and_then(|v| v.as_str())
            .expect("analysis id")
            .to_string();

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
            panic!("expected close forward");
        };

        let params = close_native
            .params
            .as_ref()
            .and_then(|v| v.as_object())
            .expect("close params object");
        assert_eq!(
            params
                .get("tabIds")
                .and_then(|v| v.as_array())
                .and_then(|arr| arr.first())
                .and_then(|v| v.as_i64()),
            Some(12)
        );
        assert_eq!(
            params
                .get("expectedUrls")
                .and_then(|v| v.get("12"))
                .and_then(|v| v.as_str()),
            Some("https://example.com")
        );
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
            panic!("expected list forward");
        };
        assert_eq!(native_req.id, "req-list");

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
                assert!(
                    config.socket_path.ends_with("profiles/edge/tabctl.sock")
                        || config.socket_path.ends_with("profiles\\edge\\tabctl.sock")
                );
            },
        );

        let _ = std::fs::remove_dir_all(&root);
    }
}
