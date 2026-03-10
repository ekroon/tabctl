use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::Path;
use url_normalize::Options as NormalizeOptions;

pub fn workspace_marker() -> &'static str {
    "tabctl-rust-workspace"
}

pub fn normalize_windows_path_string(path: &str) -> String {
    if path.is_empty() {
        return String::new();
    }

    let replaced = path.replace('/', "\\");
    let (prefix, rest) = if let Some(rest) = replaced.strip_prefix(r"\\?\UNC\") {
        (r"\\?\UNC\", rest)
    } else if let Some(rest) = replaced.strip_prefix(r"\\?\") {
        (r"\\?\", rest)
    } else if let Some(rest) = replaced.strip_prefix(r"\\.\") {
        (r"\\.\", rest)
    } else if let Some(rest) = replaced.strip_prefix(r"\\") {
        (r"\\", rest)
    } else if let Some(rest) = replaced.strip_prefix('\\') {
        (r"\", rest)
    } else {
        ("", replaced.as_str())
    };

    let had_drive_root = prefix.is_empty()
        && replaced.len() >= 3
        && replaced.as_bytes()[1] == b':'
        && replaced[2..].chars().all(|ch| ch == '\\');

    let mut segments = rest.split('\\').filter(|segment| !segment.is_empty());
    let Some(first_segment) = segments.next() else {
        return replaced;
    };

    let mut normalized = String::from(prefix);
    if first_segment.len() == 2 && first_segment.as_bytes()[1] == b':' {
        let drive = first_segment.as_bytes()[0] as char;
        normalized.push(drive.to_ascii_uppercase());
        normalized.push(':');
    } else {
        normalized.push_str(first_segment);
    }

    for segment in segments {
        if !normalized.ends_with('\\') {
            normalized.push('\\');
        }
        normalized.push_str(segment);
    }

    if had_drive_root && normalized.len() == 2 && normalized.as_bytes()[1] == b':' {
        normalized.push('\\');
    }

    normalized
}

pub fn normalize_path_for_current_platform(path: &str) -> String {
    #[cfg(windows)]
    {
        normalize_windows_path_string(path)
    }
    #[cfg(not(windows))]
    {
        path.to_string()
    }
}

pub fn path_to_platform_string(path: &Path) -> String {
    normalize_path_for_current_platform(&path.to_string_lossy())
}

pub fn windows_pipe_path(data_dir: &str) -> String {
    let normalized = normalize_windows_path_string(data_dir);
    let mut hasher = Sha256::new();
    hasher.update(normalized.as_bytes());
    let digest = hasher.finalize();
    let hash = digest[..6]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!(r"\\.\pipe\tabctl-{hash}")
}

/// Normalize a URL for deduplication and storage.
///
/// Strips protocol, `www.`, trailing slashes, fragments, and sorts query
/// parameters so that equivalent URLs produce the same key.
pub fn normalize_url(url: &str) -> String {
    let opts = NormalizeOptions {
        strip_hash: true,
        strip_protocol: true,
        ..NormalizeOptions::default()
    };
    match url_normalize::normalize_url(url, &opts) {
        Ok(normalized) => normalized,
        Err(_) => url.to_string(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TabctlConfig {
    pub config_dir: String,
    pub data_dir: String,
    pub base_data_dir: String,
    pub socket_path: String,
    pub undo_log: String,
    pub wrapper_dir: String,
    pub policy_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_profile_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Browser {
    Edge,
    Chrome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileEntry {
    pub browser: Browser,
    pub extension_id: String,
    pub node_path: String,
    pub host_path: String,
    pub data_dir: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user_data_dir: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileRegistry {
    pub default: Option<String>,
    pub profiles: HashMap<String, ProfileEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionInfo {
    pub version: String,
    pub base_version: String,
    pub git_sha: String,
    pub dirty: bool,
    pub component: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientInfo {
    pub component: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RequestEnvelope {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub action: String,
    #[serde(default)]
    pub params: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(rename = "authToken")]
    pub auth_token: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolError {
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResponseEnvelope {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ProtocolError>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NativeMessage {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ok: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ProtocolError>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SocketEndpoint {
    Unix { path: String },
    Pipe { path: String },
    Tcp { host: String, port: u16 },
}

impl SocketEndpoint {
    pub fn parse(input: &str) -> Result<Self, String> {
        let value = input.trim();
        if value.is_empty() {
            return Err("Socket endpoint cannot be empty".to_string());
        }
        if let Some(rest) = value.strip_prefix("unix://") {
            let path = if rest.starts_with('/') {
                rest.to_string()
            } else {
                format!("/{rest}")
            };
            return if path == "/" {
                Err("Unix socket endpoint requires a path".to_string())
            } else {
                Ok(Self::Unix { path })
            };
        }
        if let Some(rest) = value.strip_prefix("pipe://") {
            let normalized = if let Some(trimmed) = rest.strip_prefix('/') {
                trimmed
            } else {
                rest
            };
            return if normalized.is_empty() {
                Err("Pipe endpoint requires a path".to_string())
            } else {
                Ok(Self::Pipe {
                    path: format!(r"\\.\pipe\{normalized}"),
                })
            };
        }
        if value.starts_with(r"\\.\pipe\") {
            return Ok(Self::Pipe {
                path: value.to_string(),
            });
        }
        if let Some(rest) = value.strip_prefix("tcp://") {
            return parse_tcp(rest);
        }
        if value.contains("://") {
            return Err(format!("Unsupported socket endpoint scheme in \"{value}\""));
        }
        if Path::new(value).is_absolute() {
            return Ok(Self::Unix {
                path: value.to_string(),
            });
        }
        Err(format!("Unsupported socket endpoint format: \"{value}\""))
    }

    pub fn as_uri(&self) -> String {
        match self {
            Self::Unix { path } => format!("unix://{path}"),
            Self::Pipe { path } => {
                let suffix = path.trim_start_matches(r"\\.\pipe\");
                format!("pipe://{suffix}")
            }
            Self::Tcp { host, port } => format!("tcp://{host}:{port}"),
        }
    }
}

fn parse_tcp(value: &str) -> Result<SocketEndpoint, String> {
    let Some((host, port)) = value.rsplit_once(':') else {
        return Err("TCP endpoint must include host and port".to_string());
    };
    let host = host.trim();
    if host.is_empty() {
        return Err("TCP endpoint requires a host".to_string());
    }
    let port = port
        .trim()
        .parse::<u16>()
        .map_err(|_| "TCP endpoint has invalid port".to_string())?;
    if port == 0 {
        return Err("TCP endpoint port must be greater than zero".to_string());
    }
    Ok(SocketEndpoint::Tcp {
        host: host.to_string(),
        port,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_config_contract_shape() {
        let json = r#"{
          "configDir": "/tmp/cfg",
          "dataDir": "/tmp/data",
          "baseDataDir": "/tmp/data",
          "socketPath": "/tmp/data/tabctl.sock",
          "undoLog": "/tmp/data/undo.jsonl",
          "wrapperDir": "/tmp/data",
          "policyPath": "/tmp/cfg/policy.json",
          "activeProfileName": "edge"
        }"#;
        let cfg: TabctlConfig = serde_json::from_str(json).expect("valid config payload");
        assert_eq!(cfg.active_profile_name.as_deref(), Some("edge"));
    }

    #[test]
    fn parses_profiles_registry_shape() {
        let json = r#"{
          "default": "edge",
          "profiles": {
            "edge": {
              "browser": "edge",
              "extensionId": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
              "nodePath": "/usr/local/bin/node",
              "hostPath": "/tmp/tabctl/host.bundle.js",
              "dataDir": "/tmp/tabctl/profiles/edge"
            }
          }
        }"#;
        let registry: ProfileRegistry = serde_json::from_str(json).expect("valid registry payload");
        assert_eq!(registry.default.as_deref(), Some("edge"));
        assert_eq!(
            registry.profiles.get("edge").map(|p| p.browser.clone()),
            Some(Browser::Edge)
        );
    }

    #[test]
    fn parses_cli_request_protocol_shape() {
        let json = r#"{"id":"req-1","action":"move-tab","params":{"tabId":12}}"#;
        let request: RequestEnvelope = serde_json::from_str(json).expect("valid request payload");
        assert_eq!(request.id.as_deref(), Some("req-1"));
        assert_eq!(request.action, "move-tab");
    }

    #[test]
    fn parses_host_response_protocol_shape() {
        let json = r#"{
          "ok": true,
          "action": "move-tab",
          "requestId": "req-1",
          "component": "host",
          "version": "0.1.0",
          "data": { "summary": { "movedTabs": 1 } }
        }"#;
        let response: ResponseEnvelope =
            serde_json::from_str(json).expect("valid response payload");
        assert!(response.ok);
        assert_eq!(response.request_id.as_deref(), Some("req-1"));
    }

    #[test]
    fn serializes_response_using_camel_case_keys() {
        let response = ResponseEnvelope {
            ok: true,
            action: Some("list".to_string()),
            request_id: Some("req-2".to_string()),
            component: Some("host".to_string()),
            version: Some("0.1.0".to_string()),
            progress: Some(false),
            data: Some(serde_json::json!({"items":[] })),
            error: None,
        };
        let value = serde_json::to_value(response).expect("serialize response");
        assert_eq!(
            value.get("requestId").and_then(|v| v.as_str()),
            Some("req-2")
        );
        assert!(value.get("request_id").is_none());
    }

    #[test]
    fn parses_native_message_error_payload_shape() {
        let json = r#"{
          "id":"req-3",
          "action":"close",
          "ok":false,
          "error":{"message":"Blocked by policy","hint":"Use --confirm"}
        }"#;
        let message: NativeMessage = serde_json::from_str(json).expect("valid native message");
        assert_eq!(message.id, "req-3");
        assert_eq!(
            message.error.as_ref().and_then(|e| e.hint.as_deref()),
            Some("Use --confirm")
        );
    }

    #[test]
    fn parses_unix_endpoint_from_absolute_path() {
        let endpoint = SocketEndpoint::parse("/tmp/tabctl.sock");
        if cfg!(windows) {
            assert!(endpoint.is_err());
        } else {
            let endpoint = endpoint.expect("parse unix path");
            assert_eq!(
                endpoint,
                SocketEndpoint::Unix {
                    path: "/tmp/tabctl.sock".to_string()
                }
            );
            assert_eq!(endpoint.as_uri(), "unix:///tmp/tabctl.sock");
        }
    }

    #[test]
    fn parses_pipe_endpoint_forms() {
        let endpoint = SocketEndpoint::parse(r"\\.\pipe\tabctl-test").expect("parse raw pipe");
        assert_eq!(
            endpoint,
            SocketEndpoint::Pipe {
                path: r"\\.\pipe\tabctl-test".to_string()
            }
        );
        let endpoint = SocketEndpoint::parse("pipe://tabctl-test").expect("parse pipe uri");
        assert_eq!(endpoint.as_uri(), "pipe://tabctl-test");
    }

    #[test]
    fn parses_tcp_endpoint_form() {
        let endpoint = SocketEndpoint::parse("tcp://127.0.0.1:8008").expect("parse tcp");
        assert_eq!(
            endpoint,
            SocketEndpoint::Tcp {
                host: "127.0.0.1".to_string(),
                port: 8008
            }
        );
    }

    #[test]
    fn normalizes_unix_and_pipe_uri_paths() {
        let unix = SocketEndpoint::parse("unix://tmp/tabctl.sock").expect("parse unix uri");
        assert_eq!(
            unix,
            SocketEndpoint::Unix {
                path: "/tmp/tabctl.sock".to_string()
            }
        );
        let pipe = SocketEndpoint::parse("pipe:///tabctl-test").expect("parse pipe uri");
        assert_eq!(
            pipe,
            SocketEndpoint::Pipe {
                path: r"\\.\pipe\tabctl-test".to_string()
            }
        );
    }

    #[test]
    fn rejects_invalid_endpoint_forms() {
        assert!(SocketEndpoint::parse("tcp://127.0.0.1").is_err());
        assert!(SocketEndpoint::parse("tcp://127.0.0.1:0").is_err());
        assert!(SocketEndpoint::parse("pipe://").is_err());
        assert!(SocketEndpoint::parse("unix://").is_err());
        assert!(SocketEndpoint::parse("udp://127.0.0.1:8008").is_err());
        assert!(SocketEndpoint::parse("relative.sock").is_err());
    }

    #[test]
    fn normalize_windows_path_string_unifies_mixed_separators() {
        let normalized =
            normalize_windows_path_string(r"C:/Users/tester/AppData\\Local/tabctl/profiles/edge/");
        assert_eq!(
            normalized,
            r"C:\Users\tester\AppData\Local\tabctl\profiles\edge"
        );
    }

    #[test]
    fn normalize_windows_path_string_preserves_unc_prefix() {
        let normalized = normalize_windows_path_string(r"\\server/share\\tabctl\profiles/edge");
        assert_eq!(normalized, r"\\server\share\tabctl\profiles\edge");
    }

    #[test]
    fn windows_pipe_path_ignores_separator_style() {
        assert_eq!(
            windows_pipe_path(r"C:\Users\tester\AppData\Local\tabctl\profiles\edge"),
            windows_pipe_path(r"C:/Users/tester/AppData/Local/tabctl/profiles/edge")
        );
    }
}
