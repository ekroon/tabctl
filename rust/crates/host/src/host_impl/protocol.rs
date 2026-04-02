use serde_json::{Map, Value};
use std::collections::HashSet;
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use tabctl_shared::{NativeMessage, ResponseEnvelope, VersionInfo};

pub(super) const REQUEST_TIMEOUT_MS: u64 = 30_000;
pub(super) const MAX_NATIVE_MESSAGE_BYTES: usize = 10 * 1024 * 1024;

static ID_COUNTER: AtomicU64 = AtomicU64::new(1);

pub(super) fn host_version() -> &'static str {
    option_env!("TABCTL_HOST_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"))
}

pub(super) fn base_version() -> &'static str {
    option_env!("TABCTL_BASE_VERSION").unwrap_or(env!("CARGO_PKG_VERSION"))
}

pub(super) fn git_sha() -> &'static str {
    option_env!("TABCTL_GIT_SHA").unwrap_or("unknown")
}

pub(super) fn is_dirty() -> bool {
    option_env!("TABCTL_DIRTY")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

pub(super) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

pub(super) fn next_counter() -> u64 {
    ID_COUNTER.fetch_add(1, Ordering::Relaxed)
}

pub(super) fn create_id(prefix: &str) -> String {
    let counter = next_counter();
    format!("{prefix}-{}-{counter}", now_ms())
}

pub(super) fn local_actions() -> HashSet<&'static str> {
    HashSet::from([
        "history",
        "undo",
        "version",
        "browser-state-history",
        "browser-state-latest",
        "browser-state-events",
        "browser-state-group-history",
    ])
}

pub(super) fn undo_actions() -> HashSet<&'static str> {
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

pub(super) fn write_native_message<W: Write>(
    writer: &mut W,
    message: &NativeMessage,
) -> io::Result<()> {
    let payload = serde_json::to_vec(message)?;
    let len = payload.len() as u32;
    writer.write_all(&len.to_le_bytes())?;
    writer.write_all(&payload)?;
    writer.flush()?;
    Ok(())
}

pub(super) fn read_native_message<R: Read>(reader: &mut R) -> io::Result<Option<NativeMessage>> {
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

pub(super) fn log_line(message: &str) {
    eprintln!("[tabctl-host-rust] {message}");
}

pub(super) fn trace_enabled() -> bool {
    std::env::var("TABCTL_TRACE_IO")
        .map(|value| value == "1" || value.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
}

pub(super) fn trace_line(message: &str) {
    if trace_enabled() {
        log_line(message);
    }
}

pub(super) fn version_info_value() -> Value {
    let info = VersionInfo {
        version: host_version().to_string(),
        base_version: base_version().to_string(),
        git_sha: git_sha().to_string(),
        dirty: is_dirty(),
        component: "host".to_string(),
    };
    serde_json::to_value(info).unwrap_or(Value::Object(Map::new()))
}

pub(super) fn base_response(
    ok: bool,
    action: Option<String>,
    request_id: Option<String>,
) -> ResponseEnvelope {
    ResponseEnvelope {
        ok,
        action,
        request_id,
        component: None,
        version: None,
        progress: None,
        data: None,
        error: None,
    }
}

pub(super) fn add_host_metadata(response: &mut ResponseEnvelope) {
    response.component = Some("host".to_string());
    response.version = Some(host_version().to_string());
}

pub(super) fn value_object(input: Option<Value>) -> Map<String, Value> {
    input
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

pub(super) fn host_ping_data(native_channel_available: bool) -> Map<String, Value> {
    let mut data = Map::new();
    data.insert("now".to_string(), Value::Number(now_ms().into()));
    data.insert("component".to_string(), Value::String("host".to_string()));
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
    data.insert(
        "nativeChannelAvailable".to_string(),
        Value::Bool(native_channel_available),
    );
    data.insert("versionsInSync".to_string(), Value::Bool(false));
    data
}

pub(super) fn add_ping_metadata(mut data: Map<String, Value>) -> Map<String, Value> {
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
    let versions_in_sync = extension_version
        .as_deref()
        .map(|v| strip_dev_suffix(v) == base_version())
        .unwrap_or(false);
    data.insert("versionsInSync".to_string(), Value::Bool(versions_in_sync));
    data
}

fn strip_dev_suffix(version: &str) -> &str {
    if let Some(idx) = version.find("-dev.") {
        &version[..idx]
    } else {
        version
    }
}
