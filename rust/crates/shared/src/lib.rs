use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

pub fn workspace_marker() -> &'static str {
    "tabctl-rust-workspace"
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
}
