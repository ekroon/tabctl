use serde_json::{Map, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tabctl_shared::{ClientInfo, NativeMessage, ProtocolError, RequestEnvelope, ResponseEnvelope};

use super::browser_state;
use super::focus_store;
use super::orchestrate::{orchestration_for, Orchestration, OrchestrationContext};
use super::page_cache::{OpenTabCacheKey, PageCache};
use super::policy::{default_policy_path, Policy};
use super::protocol::{
    add_host_metadata, add_ping_metadata, base_response, create_id, host_version, local_actions,
    log_line, now_ms, trace_line, undo_actions, value_object, version_info_value,
    REQUEST_TIMEOUT_MS,
};
use super::transaction::{is_mutating_primitive, Transaction};
#[cfg(test)]
use super::undo::read_undo_records;
use super::undo::{
    append_undo_record, filter_by_retention, find_latest_undo_record_excluding, find_undo_record,
    is_pending_undo, read_undo_records_checked, unresolved_creation, RETENTION_DAYS,
};

const HISTORY_LIMIT_DEFAULT: usize = 20;
mod orchestration;
#[cfg(test)]
mod policy_tests;
#[cfg(test)]
mod privacy_tests;
mod recovery;
#[cfg(test)]
mod regressions;

#[derive(Debug)]
struct PendingRequest {
    client_id: u64,
    action: String,
    request_id: Option<String>,
    txid: Option<String>,
    created_at: u64,
    orchestration: Option<Box<dyn Orchestration>>,
    primitive: Option<String>,
}

#[derive(Debug, Clone)]
struct AnalysisRecord {
    data: Map<String, Value>,
}

#[derive(Debug)]
pub(super) struct HostState {
    pending: HashMap<String, PendingRequest>,
    analyses: HashMap<String, AnalysisRecord>,
    undo_log: PathBuf,
    state_db_path: PathBuf,
    focus_db_path: PathBuf,
    page_cache_path: PathBuf,
    profile_name: Option<String>,
    native_channel_available: bool,
    transactions: HashMap<String, Transaction>,
    policy: Arc<Policy>,
    policy_error: Option<String>,
    policy_path: PathBuf,
}

#[derive(Debug)]
pub(super) enum HostEffect {
    SendNative(NativeMessage),
    Respond {
        client_id: u64,
        payload: ResponseEnvelope,
    },
}

impl HostState {
    fn ingest_snapshot_response(&self, snapshot: &Value) {
        let payload = serde_json::json!({
            "reason": "snapshot",
            "recordedAt": now_ms(),
            "snapshot": snapshot,
        });
        if let Err(err) =
            browser_state::ingest_sync(&self.state_db_path, self.profile_name.as_deref(), &payload)
        {
            log_line(&format!("browser-state snapshot ingest failed: {err}"));
        }
    }

    fn ingest_page_cache_capture(&self, payload: &Value) -> bool {
        match self.try_ingest_page_cache_capture(payload) {
            Ok(available) => available,
            Err(err) => {
                log_line(&format!("page-cache capture ingest failed: {err}"));
                false
            }
        }
    }

    fn try_ingest_page_cache_capture(&self, payload: &Value) -> Result<bool, String> {
        let Some(root) = payload.as_object() else {
            return Ok(false);
        };
        let Some(tab) = root.get("tab").and_then(Value::as_object) else {
            return Ok(false);
        };
        let Some(extraction) = root.get("extraction").and_then(Value::as_object) else {
            return Ok(false);
        };

        if extraction.get("status").and_then(Value::as_str) != Some("READ") {
            return Ok(false);
        }

        let Some(html) = extraction.get("html").and_then(Value::as_str) else {
            return Ok(false);
        };
        if html.is_empty() {
            return Ok(false);
        }

        let Some(tab_id) = tab.get("tabId").and_then(Value::as_i64) else {
            return Ok(false);
        };
        let Some(url) = tab.get("url").and_then(Value::as_str) else {
            return Ok(false);
        };
        if url.is_empty() {
            return Ok(false);
        }

        let incognito = tab
            .get("incognito")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let truncated_html = extraction
            .get("truncatedHtml")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if incognito || truncated_html || is_non_scriptable_url(url) {
            return Ok(false);
        }

        let source_html_chars = extraction
            .get("sourceHtmlChars")
            .and_then(Value::as_i64)
            .unwrap_or_else(|| html.chars().count() as i64);
        let source_text_chars = extraction
            .get("sourceTextChars")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let document_ready_state = extraction.get("documentReadyState").and_then(Value::as_str);
        let captured_at = root
            .get("capturedAt")
            .and_then(Value::as_i64)
            .unwrap_or_else(|| now_ms() as i64);
        let title = tab.get("title").and_then(Value::as_str);

        let open_tabs = page_cache_open_tabs_from_payload(root);
        PageCache::store_success_file(
            &self.page_cache_path,
            self.profile_name.as_deref(),
            tab_id,
            url,
            title,
            html,
            source_html_chars,
            source_text_chars,
            document_ready_state,
            truncated_html,
            incognito,
            captured_at,
            open_tabs.as_deref(),
        )
    }

    fn page_cache_status(&self, payload: &Value) -> Value {
        let Some((tab_id, url, incognito, discarded)) = page_cache_tab_request(payload) else {
            return page_cache_status_value(None, None, false);
        };
        if incognito || discarded || is_non_scriptable_url(&url) {
            return page_cache_status_value(Some(tab_id), Some(&url), false);
        }
        let available = match PageCache::exact_file_available(
            &self.page_cache_path,
            self.profile_name.as_deref(),
            tab_id,
            &url,
        ) {
            Ok(available) => available,
            Err(err) => {
                log_line(&format!("page-cache status failed: {err}"));
                false
            }
        };
        page_cache_status_value(Some(tab_id), Some(&url), available)
    }

    fn page_cache_status_effect(&self, id: String, payload: &Value, available: bool) -> HostEffect {
        let Some((tab_id, url, _, _)) = page_cache_tab_request(payload) else {
            return HostEffect::SendNative(NativeMessage {
                id,
                action: None,
                ok: Some(true),
                progress: None,
                params: None,
                data: Some(page_cache_status_value(None, None, false)),
                error: None,
            });
        };
        HostEffect::SendNative(NativeMessage {
            id,
            action: None,
            ok: Some(true),
            progress: None,
            params: None,
            data: Some(page_cache_status_value(Some(tab_id), Some(&url), available)),
            error: None,
        })
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) fn new(
        undo_log: PathBuf,
        focus_db_path: PathBuf,
        profile_name: Option<String>,
    ) -> Self {
        Self::new_with_native_channel(undo_log, focus_db_path, profile_name, true)
    }

    pub(super) fn new_with_native_channel(
        undo_log: PathBuf,
        focus_db_path: PathBuf,
        profile_name: Option<String>,
        native_channel_available: bool,
    ) -> Self {
        let page_cache_path = undo_log
            .parent()
            .map(|parent| parent.join("page-cache"))
            .unwrap_or_else(|| PathBuf::from("page-cache"));
        Self::new_with_native_channel_and_page_cache(
            undo_log,
            focus_db_path,
            page_cache_path,
            profile_name,
            native_channel_available,
        )
    }

    pub(super) fn new_with_native_channel_and_page_cache(
        undo_log: PathBuf,
        focus_db_path: PathBuf,
        page_cache_path: PathBuf,
        profile_name: Option<String>,
        native_channel_available: bool,
    ) -> Self {
        let state_db_path = undo_log
            .parent()
            .map(|parent| parent.join("state.db"))
            .unwrap_or_else(|| PathBuf::from("state.db"));
        let policy_path = if cfg!(test) {
            undo_log
                .parent()
                .unwrap_or_else(|| std::path::Path::new("."))
                .join("policy.json")
        } else {
            default_policy_path()
        };
        let policy = Policy::load(&policy_path);
        let policy_error = policy.as_ref().err().cloned();
        Self {
            pending: HashMap::new(),
            analyses: HashMap::new(),
            undo_log,
            state_db_path,
            focus_db_path,
            page_cache_path,
            profile_name,
            native_channel_available,
            transactions: HashMap::new(),
            policy: Arc::new(policy.unwrap_or_default()),
            policy_error,
            policy_path,
        }
    }

    fn orchestration_context(&self) -> OrchestrationContext {
        OrchestrationContext {
            page_cache_path: Some(self.page_cache_path.clone()),
            profile_name: self.profile_name.clone(),
            policy: self.policy.clone(),
        }
    }

    fn forward_to_extension(
        &mut self,
        client_id: u64,
        request: &RequestEnvelope,
        txid: Option<String>,
    ) -> Vec<HostEffect> {
        self.forward_to_extension_with_orch(client_id, request, txid, None)
    }

    pub(super) fn collect_timed_out_requests(&mut self) -> Vec<HostEffect> {
        let expired: Vec<String> = self
            .pending
            .iter()
            .filter(|(_, pending)| now_ms().saturating_sub(pending.created_at) > REQUEST_TIMEOUT_MS)
            .map(|(message_id, _)| message_id.clone())
            .collect();

        let mut effects = Vec::new();
        for message_id in expired {
            if let Some(pending) = self.pending.remove(&message_id) {
                effects.push(self.timeout_effect(pending, message_id));
            }
        }
        effects
    }

    pub(super) fn fail_pending_request(
        &mut self,
        message_id: &str,
        message: String,
        hint: Option<String>,
    ) -> Option<HostEffect> {
        let pending = self.pending.remove(message_id)?;
        Some(self.error_effect(pending, message_id.to_string(), message, hint))
    }

    pub(super) fn fail_all_pending_requests(
        &mut self,
        message: String,
        hint: Option<String>,
    ) -> Vec<HostEffect> {
        self.native_channel_available = false;
        let pending: Vec<_> = self.pending.drain().collect();
        pending
            .into_iter()
            .map(|(id, pending)| self.error_effect(pending, id, message.clone(), hint.clone()))
            .collect()
    }

    fn forward_to_extension_with_orch(
        &mut self,
        client_id: u64,
        request: &RequestEnvelope,
        txid: Option<String>,
        orchestration: Option<Box<dyn Orchestration>>,
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
                request_id: Some(request_id.clone()),
                txid,
                created_at: now_ms(),
                orchestration,
                primitive: None,
            },
        );
        trace_line(&format!(
            "pending insert: client_id={} request_id={} action={}",
            client_id, request_id, request.action
        ));

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

    pub(super) fn handle_cli_request(
        &mut self,
        client_id: u64,
        mut request: RequestEnvelope,
    ) -> Vec<HostEffect> {
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
        if action.starts_with("p:") {
            let mut resp = base_response(false, Some(action), request.id);
            resp.error = Some(ProtocolError {
                message: "Browser primitives are internal-only".into(),
                hint: Some(
                    "Use a scoped browser command so protection and recovery are enforced.".into(),
                ),
            });
            return vec![HostEffect::Respond {
                client_id,
                payload: resp,
            }];
        }
        if undo_actions().contains(action.as_str()) || action == "analyze" {
            let policy = Policy::load(&self.policy_path);
            self.policy_error = policy.as_ref().err().cloned();
            self.policy = Arc::new(policy.unwrap_or_default());
        }

        if !self.native_channel_available
            && (!local_actions().contains(action.as_str()) || action == "undo")
        {
            let mut resp = base_response(false, Some(action), request.id);
            resp.error = Some(ProtocolError {
                message: "Native browser channel unavailable".to_string(),
                hint: Some(
                    "The host is running without an attached browser native messaging channel, so browser-backed actions cannot complete.".to_string(),
                ),
            });
            return vec![HostEffect::Respond {
                client_id,
                payload: resp,
            }];
        }

        if action == "version" {
            let mut resp = base_response(true, Some(action), request.id);
            add_host_metadata(&mut resp);
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
            let mut records = match read_undo_records_checked(&self.undo_log) {
                Ok(records) => filter_by_retention(records, RETENTION_DAYS),
                Err(message) => {
                    let mut resp = base_response(false, Some(action), request.id);
                    resp.error = Some(ProtocolError {
                        message,
                        hint: None,
                    });
                    return vec![HostEffect::Respond {
                        client_id,
                        payload: resp,
                    }];
                }
            };
            for record in &mut records {
                let active = record
                    .get("txid")
                    .and_then(Value::as_str)
                    .is_some_and(|id| self.transactions.contains_key(id));
                if !active && unresolved_creation(record) {
                    record.insert("status".into(), Value::String("recovery_uncertain".into()));
                }
            }
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

        if action == "browser-state-history" {
            let limit = request
                .params
                .get("limit")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);
            let data = browser_state::list_history(
                &self.state_db_path,
                self.profile_name.as_deref(),
                limit,
            );
            let mut resp = base_response(data.is_ok(), Some(action), request.id);
            match data {
                Ok(data) => resp.data = Some(data),
                Err(message) => {
                    resp.error = Some(ProtocolError {
                        message,
                        hint: None,
                    })
                }
            }
            return vec![HostEffect::Respond {
                client_id,
                payload: resp,
            }];
        }

        if action == "browser-state-latest" {
            let data =
                browser_state::latest_snapshot(&self.state_db_path, self.profile_name.as_deref());
            let mut resp = base_response(data.is_ok(), Some(action), request.id);
            match data {
                Ok(data) => resp.data = Some(data),
                Err(message) => {
                    resp.error = Some(ProtocolError {
                        message,
                        hint: None,
                    })
                }
            }
            return vec![HostEffect::Respond {
                client_id,
                payload: resp,
            }];
        }

        if action == "browser-state-events" {
            let limit = request
                .params
                .get("limit")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);
            let kind = request.params.get("kind").and_then(|v| v.as_str());
            let data = browser_state::list_events(
                &self.state_db_path,
                self.profile_name.as_deref(),
                limit,
                kind,
            );
            let mut resp = base_response(data.is_ok(), Some(action), request.id);
            match data {
                Ok(data) => resp.data = Some(data),
                Err(message) => {
                    resp.error = Some(ProtocolError {
                        message,
                        hint: None,
                    })
                }
            }
            return vec![HostEffect::Respond {
                client_id,
                payload: resp,
            }];
        }

        if action == "browser-state-group-history" {
            let limit = request
                .params
                .get("limit")
                .and_then(|v| v.as_u64())
                .map(|v| v as usize);
            let title = request.params.get("title").and_then(|v| v.as_str());
            let logical_group_id = request
                .params
                .get("logicalGroupId")
                .and_then(|v| v.as_str());
            let data = browser_state::list_group_history(
                &self.state_db_path,
                self.profile_name.as_deref(),
                limit,
                title,
                logical_group_id,
            );
            let mut resp = base_response(data.is_ok(), Some(action), request.id);
            match data {
                Ok(data) => resp.data = Some(data),
                Err(message) => {
                    resp.error = Some(ProtocolError {
                        message,
                        hint: None,
                    })
                }
            }
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

            if txid
                .as_ref()
                .is_some_and(|id| self.transactions.contains_key(id))
            {
                let mut resp = base_response(false, Some(action), request.id);
                resp.error = Some(ProtocolError {
                    message: "Cannot undo an active transaction".into(),
                    hint: Some("Wait for the original request to finish. Orphaned records can be recovered after the original host process exits.".into()),
                });
                return vec![HostEffect::Respond {
                    client_id,
                    payload: resp,
                }];
            }
            let record = if let Some(tx) = txid {
                find_undo_record(&self.undo_log, &tx)
            } else {
                find_latest_undo_record_excluding(
                    &self.undo_log,
                    &self.transactions.keys().cloned().collect(),
                )
            };
            let record = match record {
                Ok(record) => record,
                Err(message) => {
                    let mut resp = base_response(false, Some(action), request.id);
                    resp.error = Some(ProtocolError {
                        message,
                        hint: None,
                    });
                    return vec![HostEffect::Respond {
                        client_id,
                        payload: resp,
                    }];
                }
            };

            let Some(mut record) = record else {
                let mut resp = base_response(false, Some(action), request.id);
                resp.error = Some(ProtocolError {
                    message: "Undo record not found".to_string(),
                    hint: Some("Inspect history: active and uncertain transactions are excluded from latest undo.".into()),
                });
                return vec![HostEffect::Respond {
                    client_id,
                    payload: resp,
                }];
            };

            if unresolved_creation(&record)
                || matches!(
                    record.get("status").and_then(Value::as_str),
                    Some("recovery_uncertain" | "undo_uncertain")
                )
            {
                let mut resp = base_response(false, Some(action), request.id);
                resp.error = Some(ProtocolError {
                    message: "Recovery is uncertain: a creation reply did not establish the browser effects and IDs".into(),
                    hint: Some("Preserve the journal and inspect browser state manually. Automatic undo cannot safely guess created IDs or report successful restoration.".into()),
                });
                return vec![HostEffect::Respond {
                    client_id,
                    payload: resp,
                }];
            }
            if !is_pending_undo(&record) {
                let mut resp = base_response(false, Some(action), request.id);
                resp.error = Some(ProtocolError {
                    message: "Transaction already undone or undo in progress".into(),
                    hint: None,
                });
                return vec![HostEffect::Respond {
                    client_id,
                    payload: resp,
                }];
            }
            let undo_txid = record
                .get("txid")
                .and_then(Value::as_str)
                .map(str::to_owned);
            record.insert("status".into(), Value::String("undoing".into()));
            if let Err(message) = append_undo_record(&self.undo_log, &record) {
                let mut resp = base_response(false, Some(action), request.id);
                resp.error = Some(ProtocolError {
                    message: format!("Cannot checkpoint undo: {message}"),
                    hint: None,
                });
                return vec![HostEffect::Respond {
                    client_id,
                    payload: resp,
                }];
            }
            let undo_params = Value::Object(Map::from_iter([(
                "record".to_string(),
                Value::Object(record),
            )]));
            if let Some(mut orch) =
                orchestration_for("undo", &undo_params, &self.orchestration_context())
            {
                let step = orch.start();
                return self
                    .process_orch_step(client_id, "undo", request.id, undo_txid, step, orch);
            }
            let undo_request = RequestEnvelope {
                id: request.id,
                action: "undo".to_string(),
                params: undo_params,
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
            for field in ["confirmed", "confirm", "dryRun"] {
                if let Some(value) = request.params.get(field) {
                    params.insert(field.into(), value.clone());
                }
            }
            // Fall through to orchestration below with enriched params
            request = RequestEnvelope {
                id: request.id,
                action: action.clone(),
                params: Value::Object(params),
                auth_token: None,
            };
        }

        // Check for orchestration — new primitive-based path
        // Must come before undo_actions legacy forward so migrated commands
        // use the orchestration path. Undo-tracked orchestrated commands get
        // a txid generated here.
        let mutating_analysis = action == "analyze"
            && request.params["dedupe"].as_bool() == Some(true)
            && request
                .params
                .get("confirmed")
                .or_else(|| request.params.get("confirm"))
                .and_then(Value::as_bool)
                == Some(true)
            && request.params["dryRun"].as_bool() != Some(true);
        if (undo_actions().contains(action.as_str()) || mutating_analysis)
            && self.policy_error.is_some()
        {
            let mut resp = base_response(false, Some(action), request.id);
            resp.error = Some(ProtocolError {
                message: self.policy_error.clone().unwrap(),
                hint: None,
            });
            return vec![HostEffect::Respond {
                client_id,
                payload: resp,
            }];
        }
        if let Some(mut orch) =
            orchestration_for(&action, &request.params, &self.orchestration_context())
        {
            let txid = if undo_actions().contains(action.as_str()) || mutating_analysis {
                Some(create_id("tx"))
            } else {
                None
            };
            if let Some(txid) = txid.as_ref() {
                self.transactions
                    .insert(txid.clone(), Transaction::new(txid, &action));
            }
            let step = orch.start();
            return self.process_orch_step(client_id, &action, request.id, txid, step, orch);
        }

        if undo_actions().contains(action.as_str()) {
            return self.forward_to_extension(client_id, &request, Some(create_id("tx")));
        }

        self.forward_to_extension(client_id, &request, None)
    }

    pub(super) fn handle_native_message(&mut self, message: NativeMessage) -> Vec<HostEffect> {
        let message_id = message.id.clone();

        if !self.pending.contains_key(&message_id) {
            if message.ok == Some(false) {
                log_line(&format!(
                    "Rejected failed unsolicited native message {}: {:?}",
                    message.action.as_deref().unwrap_or("unknown"),
                    message.error
                ));
                return Vec::new();
            }
            if message.action.as_deref() == Some("browser-state-sync") {
                let mut payload = message.data.unwrap_or(Value::Object(Map::new()));
                if let Some(snapshot) = payload.get_mut("snapshot") {
                    if let Err(err) = focus_store::enrich_snapshot(
                        &self.focus_db_path,
                        snapshot,
                        self.profile_name.as_deref(),
                    ) {
                        log_line(&format!("browser-state focus enrichment failed: {err}"));
                    }
                }
                if let Err(err) = browser_state::ingest_sync(
                    &self.state_db_path,
                    self.profile_name.as_deref(),
                    &payload,
                ) {
                    log_line(&format!("browser-state sync ingest failed: {err}"));
                }
            } else if message.action.as_deref() == Some("page-cache-capture") {
                let payload = message.data.unwrap_or(Value::Object(Map::new()));
                let available = self.ingest_page_cache_capture(&payload);
                return vec![self.page_cache_status_effect(message_id, &payload, available)];
            } else if message.action.as_deref() == Some("page-cache-status") {
                let payload = message.data.unwrap_or(Value::Object(Map::new()));
                let data = self.page_cache_status(&payload);
                return vec![HostEffect::SendNative(NativeMessage {
                    id: message_id,
                    action: None,
                    ok: Some(true),
                    progress: None,
                    params: None,
                    data: Some(data),
                    error: None,
                })];
            }
            return Vec::new();
        }

        // Timeout check
        if let Some(pending) = self.pending.get(&message_id) {
            if now_ms().saturating_sub(pending.created_at) > REQUEST_TIMEOUT_MS {
                trace_line(&format!(
                    "pending timeout: request_id={} action={}",
                    message_id, pending.action
                ));
                let timed_out = self.pending.remove(&message_id).expect("pending exists");
                return vec![self.timeout_effect(timed_out, message_id)];
            }
        }

        // Progress passthrough (don't remove pending)
        if message.progress.unwrap_or(false) {
            let Some(pending) = self.pending.get(&message_id) else {
                return Vec::new();
            };
            let resp_id = pending
                .request_id
                .clone()
                .unwrap_or_else(|| message_id.clone());
            let mut resp = base_response(true, Some(pending.action.clone()), Some(resp_id));
            resp.progress = Some(true);
            resp.data = message.data;
            return vec![HostEffect::Respond {
                client_id: pending.client_id,
                payload: resp,
            }];
        }

        // Remove pending for main processing
        let Some(mut pending) = self.pending.remove(&message_id) else {
            return Vec::new();
        };
        if pending.action == "undo"
            && pending
                .txid
                .as_ref()
                .is_some_and(|id| self.transactions.contains_key(id))
        {
            return vec![self.error_effect(
                pending,
                message_id,
                "Cannot undo an active transaction".into(),
                None,
            )];
        }

        // Extension error — abort orchestration if active
        if !message.ok.unwrap_or(false) {
            let error = message.error.unwrap_or(ProtocolError {
                message: "Unknown error".to_string(),
                hint: None,
            });
            return vec![self.error_effect(pending, message_id, error.message, error.hint)];
        }

        let message_data = value_object(message.data.clone());

        // Orchestration path — feed response to the state machine
        if let Some(mut orch) = pending.orchestration.take() {
            // Enrich snapshot responses with focus store data
            let mut response_data = message.data.clone().unwrap_or(Value::Object(message_data));
            if response_data.get("windows").is_some() {
                let _ = focus_store::enrich_snapshot(
                    &self.focus_db_path,
                    &mut response_data,
                    self.profile_name.as_deref(),
                );
                self.ingest_snapshot_response(&response_data);
                if pending.action != "undo" {
                    if let Some(tx) = pending
                        .txid
                        .as_ref()
                        .and_then(|id| self.transactions.get_mut(id))
                    {
                        if tx.snapshot.is_null() {
                            tx.snapshot = response_data.clone();
                        }
                    }
                }
            }
            if let Some(primitive) = pending
                .primitive
                .as_ref()
                .filter(|action| pending.action != "undo" && is_mutating_primitive(action))
            {
                if let Some(tx) = pending
                    .txid
                    .as_ref()
                    .and_then(|id| self.transactions.get_mut(id))
                {
                    if let Err(message) = tx.acknowledge(primitive, &response_data, &self.undo_log)
                    {
                        return vec![self.error_effect(pending, message_id, message, None)];
                    }
                }
            }
            let step = orch.step(response_data);
            if pending.action == "undo" {
                if let Some(txid) = pending.txid.as_ref() {
                    if let Some(checkpoint) = orch.recovery_checkpoint() {
                        let record = find_undo_record(&self.undo_log, txid).and_then(|record| {
                            record.ok_or_else(|| {
                                "Undo record disappeared before recovery checkpoint".into()
                            })
                        });
                        let mut record = match record {
                            Ok(record) => record,
                            Err(message) => {
                                return vec![self.error_effect(
                                    pending,
                                    message_id,
                                    message,
                                    Some("Inspect the journal before retrying restoration.".into()),
                                )]
                            }
                        };
                        record.insert("undo".into(), checkpoint);
                        if let Err(message) = append_undo_record(&self.undo_log, &record) {
                            return vec![self.error_effect(
                                pending,
                                message_id,
                                format!(
                                    "Restoration applied but cannot checkpoint recovery: {message}"
                                ),
                                Some("Do not retry until browser state is inspected.".into()),
                            )];
                        }
                    }
                }
            }
            return self.process_orch_step(
                pending.client_id,
                &pending.action.clone(),
                pending.request_id,
                pending.txid,
                step,
                orch,
            );
        }

        // Legacy path — no orchestration
        let legacy_response_data = if pending.action == "snapshot" {
            let mut response_data = message
                .data
                .clone()
                .unwrap_or(Value::Object(message_data.clone()));
            if response_data.get("windows").is_some() {
                let _ = focus_store::enrich_snapshot(
                    &self.focus_db_path,
                    &mut response_data,
                    self.profile_name.as_deref(),
                );
                self.ingest_snapshot_response(&response_data);
            }
            Some(response_data)
        } else {
            None
        };

        if pending.action == "ping" {
            let data = add_ping_metadata(message_data);
            let mut resp = base_response(true, Some("ping".to_string()), Some(message_id));
            add_host_metadata(&mut resp);
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
                if let Err(message) = append_undo_record(&self.undo_log, &record) {
                    return vec![self.error_effect(
                        pending,
                        message_id,
                        format!("Mutation applied but cannot persist undo: {message}"),
                        None,
                    )];
                }
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
        // Preserve original data shape (arrays, objects, etc.)
        resp.data = Some(
            legacy_response_data
                .unwrap_or_else(|| message.data.unwrap_or(Value::Object(message_data))),
        );
        vec![HostEffect::Respond {
            client_id: pending.client_id,
            payload: resp,
        }]
    }
}

fn page_cache_open_tabs_from_payload(root: &Map<String, Value>) -> Option<Vec<OpenTabCacheKey>> {
    for key in ["openTabs", "tabs"] {
        if let Some(tabs) = root.get(key).and_then(Value::as_array) {
            let open_tabs = collect_page_cache_open_tabs(tabs);
            if !open_tabs.is_empty() {
                return Some(open_tabs);
            }
        }
    }

    root.get("snapshot").and_then(|snapshot| {
        let open_tabs = collect_page_cache_open_tabs_from_snapshot(snapshot);
        (!open_tabs.is_empty()).then_some(open_tabs)
    })
}

fn collect_page_cache_open_tabs(tabs: &[Value]) -> Vec<OpenTabCacheKey> {
    tabs.iter()
        .filter_map(|tab| {
            let tab = tab.as_object()?;
            let tab_id = tab.get("tabId").and_then(Value::as_i64)?;
            let url = tab.get("url").and_then(Value::as_str)?;
            (!url.is_empty()).then(|| OpenTabCacheKey::new(tab_id, url))
        })
        .collect()
}

fn collect_page_cache_open_tabs_from_snapshot(snapshot: &Value) -> Vec<OpenTabCacheKey> {
    let Some(snapshot) = snapshot.as_object() else {
        return Vec::new();
    };
    if let Some(tabs) = snapshot.get("tabs").and_then(Value::as_array) {
        return collect_page_cache_open_tabs(tabs);
    }
    snapshot
        .get("windows")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|window| window.get("tabs").and_then(Value::as_array))
        .flat_map(|tabs| collect_page_cache_open_tabs(tabs))
        .collect()
}

fn page_cache_tab_request(payload: &Value) -> Option<(i64, String, bool, bool)> {
    let root = payload.as_object()?;
    let tab = root.get("tab").and_then(Value::as_object)?;
    let tab_id = tab.get("tabId").and_then(Value::as_i64)?;
    let url = tab.get("url").and_then(Value::as_str)?.to_string();
    if url.is_empty() {
        return None;
    }
    let incognito = tab
        .get("incognito")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let discarded = tab
        .get("discarded")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Some((tab_id, url, incognito, discarded))
}

fn page_cache_status_value(tab_id: Option<i64>, url: Option<&str>, available: bool) -> Value {
    serde_json::json!({
        "tabId": tab_id,
        "url": url,
        "available": available,
    })
}

fn is_non_scriptable_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    !(lower.starts_with("http://") || lower.starts_with("https://"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Map, Value};
    use std::fs;
    use std::path::Path;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant};

    fn state_path(name: &str) -> PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let id = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("target")
            .join("state-tests")
            .join(format!("{}-{}-{}", name, std::process::id(), id));
        let _ = fs::remove_dir_all(&path);
        path
    }

    fn test_state() -> HostState {
        let path = state_path("default");
        HostState::new(path.join("undo.jsonl"), path.join("focus.db"), None)
    }

    fn mutation_request() -> RequestEnvelope {
        RequestEnvelope {
            id: Some("close-request".into()),
            action: "close".into(),
            params: serde_json::json!({"tabIds":[1],"confirmed":true}),
            auth_token: None,
        }
    }

    fn native_reply(id: String, ok: bool, data: Value) -> NativeMessage {
        NativeMessage {
            id,
            action: None,
            ok: Some(ok),
            progress: None,
            params: None,
            data: Some(data),
            error: None,
        }
    }

    #[test]
    fn failed_primitive_keeps_scoped_write_ahead_recovery() {
        let mut state = test_state();
        let effects = state.handle_cli_request(1, mutation_request());
        let [HostEffect::SendNative(snapshot)] = effects.as_slice() else {
            panic!("expected snapshot")
        };
        let snapshot_id = snapshot.id.clone();
        let effects = state.handle_native_message(native_reply(snapshot_id, true, serde_json::json!({
            "windows":[{"windowId":7,"tabs":[{"tabId":1,"url":"https://a.example","index":0},{"tabId":2,"url":"https://b.example","index":1}]}]
        })));
        let [HostEffect::SendNative(remove)] = effects.as_slice() else {
            panic!("expected removal")
        };
        let records = read_undo_records(&state.undo_log);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0]["status"], "in_progress");
        assert_eq!(records[0]["undo"]["tabs"].as_array().unwrap().len(), 1);
        let effects =
            state.handle_native_message(native_reply(remove.id.clone(), false, Value::Null));
        let [HostEffect::Respond { payload, .. }] = effects.as_slice() else {
            panic!("expected failure")
        };
        assert!(!payload.ok);
        assert!(payload.data.as_ref().unwrap()["txid"].as_str().is_some());
        let records = read_undo_records(&state.undo_log);
        assert_eq!(records[0]["status"], "failed");
        assert_eq!(records[0]["inFlight"]["action"], "p:tab-remove");
        assert!(state.pending.is_empty());
        fs::remove_dir_all(state.undo_log.parent().unwrap()).unwrap();
    }

    #[test]
    fn failed_recovery_write_blocks_mutation_before_send() {
        let mut state = test_state();
        fs::create_dir_all(&state.undo_log).unwrap();
        let effects = state.handle_cli_request(1, mutation_request());
        let [HostEffect::SendNative(snapshot)] = effects.as_slice() else {
            panic!("expected snapshot")
        };
        let effects = state.handle_native_message(native_reply(
            snapshot.id.clone(),
            true,
            serde_json::json!({
                "windows":[{"windowId":7,"tabs":[{"tabId":1,"url":"https://a.example","index":0}]}]
            }),
        ));
        let [HostEffect::Respond { payload, .. }] = effects.as_slice() else {
            panic!("must not issue remove")
        };
        assert!(!payload.ok);
        assert!(payload
            .error
            .as_ref()
            .unwrap()
            .message
            .contains("persist undo"));
        fs::remove_dir_all(state.undo_log.parent().unwrap()).unwrap();
    }

    #[test]
    fn ping_without_browser_is_not_healthy() {
        let mut state = test_state();
        state.native_channel_available = false;
        let effects = state.handle_cli_request(
            1,
            RequestEnvelope {
                action: "ping".into(),
                id: Some("ping".into()),
                params: serde_json::json!({}),
                auth_token: None,
            },
        );
        let [HostEffect::Respond { payload, .. }] = effects.as_slice() else {
            panic!("expected failure")
        };
        assert!(!payload.ok);
        assert_eq!(
            payload.error.as_ref().unwrap().message,
            "Native browser channel unavailable"
        );
    }

    #[test]
    fn external_primitives_cannot_bypass_policy_and_recovery() {
        let mut state = test_state();
        for action in ["p:snapshot", "p:tab-remove", "p:window-remove"] {
            let effects = state.handle_cli_request(
                1,
                RequestEnvelope {
                    action: action.into(),
                    id: None,
                    params: serde_json::json!({"tabIds":[1],"windowId":7}),
                    auth_token: None,
                },
            );
            let [HostEffect::Respond { payload, .. }] = effects.as_slice() else {
                panic!("must not forward primitive")
            };
            assert!(!payload.ok);
            assert_eq!(
                payload.error.as_ref().unwrap().message,
                "Browser primitives are internal-only"
            );
        }
        assert!(state.pending.is_empty());
    }

    fn page_cache_test_state(name: &str) -> (HostState, PathBuf) {
        let path = state_path(name);
        let page_cache_path = path.join("page-cache");
        (
            HostState::new_with_native_channel_and_page_cache(
                path.join("undo.jsonl"),
                path.join("focus.db"),
                page_cache_path.clone(),
                Some("work".to_string()),
                true,
            ),
            page_cache_path,
        )
    }

    fn page_cache_capture(
        status: &str,
        incognito: bool,
        truncated_html: bool,
        html: &str,
    ) -> NativeMessage {
        NativeMessage {
            id: "unsolicited-page-cache".to_string(),
            action: Some("page-cache-capture".to_string()),
            ok: Some(true),
            progress: None,
            params: None,
            data: Some(serde_json::json!({
                "reason": "activation",
                "capturedAt": 12345,
                "tab": {
                    "tabId": 42,
                    "windowId": 7,
                    "index": 0,
                    "url": "https://example.com/page#section",
                    "title": "Example Page",
                    "incognito": incognito
                },
                "extraction": {
                    "status": status,
                    "html": html,
                    "sourceHtmlChars": 99,
                    "sourceTextChars": 12,
                    "documentReadyState": "complete",
                    "truncatedHtml": truncated_html
                },
                "snapshot": {
                    "windows": [{
                        "tabs": [{
                            "tabId": 42,
                            "url": "https://example.com/page#section"
                        }]
                    }]
                }
            })),
            error: None,
        }
    }

    fn cached_entry(path: &Path) -> Option<super::super::page_cache::PageCacheLookup> {
        PageCache::load(path).lookup_open_tab(Some("work"), 42, "https://example.com/page#section")
    }

    fn page_cache_status_request(tab_id: i64, url: &str) -> NativeMessage {
        NativeMessage {
            id: "page-cache-status-1".to_string(),
            action: Some("page-cache-status".to_string()),
            ok: Some(true),
            progress: None,
            params: None,
            data: Some(serde_json::json!({
                "reason": "test",
                "tab": {
                    "tabId": tab_id,
                    "url": url,
                    "incognito": false,
                    "discarded": false
                }
            })),
            error: None,
        }
    }

    fn assert_cache_status_effect(effects: &[HostEffect], available: bool) {
        let [HostEffect::SendNative(message)] = effects else {
            panic!("expected one native status response");
        };
        assert_eq!(message.action, None);
        assert_eq!(message.ok, Some(true));
        let data = message.data.as_ref().expect("status data");
        assert_eq!(data["tabId"], 42);
        assert_eq!(data["url"], "https://example.com/page#section");
        assert_eq!(data["available"], available);
    }

    #[test]
    fn unsolicited_page_cache_capture_stores_valid_read() {
        let (mut state, page_cache_path) = page_cache_test_state("valid-capture");
        let effects = state.handle_native_message(page_cache_capture(
            "READ",
            false,
            false,
            "<html>ok</html>",
        ));

        assert_cache_status_effect(&effects, true);
        let entry = cached_entry(&page_cache_path).expect("cached page capture");
        assert_eq!(entry.entry.title.as_deref(), Some("Example Page"));
        assert_eq!(entry.entry.html, "<html>ok</html>");
        assert_eq!(entry.entry.source_html_chars, 99);
        assert_eq!(entry.entry.source_text_chars, 12);
        assert_eq!(
            entry.entry.document_ready_state.as_deref(),
            Some("complete")
        );
        assert_eq!(entry.entry.captured_at, 12345);
    }

    #[test]
    fn unsolicited_page_cache_capture_skips_invalid_status() {
        let (mut state, page_cache_path) = page_cache_test_state("invalid-status");
        let effects = state.handle_native_message(page_cache_capture(
            "ERROR",
            false,
            false,
            "<html>ok</html>",
        ));

        assert_cache_status_effect(&effects, false);
        assert!(cached_entry(&page_cache_path).is_none());
    }

    #[test]
    fn unsolicited_page_cache_capture_skips_incognito() {
        let (mut state, page_cache_path) = page_cache_test_state("incognito");
        let effects =
            state.handle_native_message(page_cache_capture("READ", true, false, "<html>ok</html>"));

        assert_cache_status_effect(&effects, false);
        assert!(cached_entry(&page_cache_path).is_none());
    }

    #[test]
    fn unsolicited_page_cache_capture_skips_truncated_html() {
        let (mut state, page_cache_path) = page_cache_test_state("truncated");
        let effects =
            state.handle_native_message(page_cache_capture("READ", false, true, "<html>ok</html>"));

        assert_cache_status_effect(&effects, false);
        assert!(cached_entry(&page_cache_path).is_none());
    }

    #[test]
    fn unsolicited_page_cache_capture_save_failure_is_nonfatal() {
        let (mut state, page_cache_path) = page_cache_test_state("save-failure");
        fs::create_dir_all(page_cache_path.parent().expect("parent")).expect("create parent");
        fs::write(&page_cache_path, b"not a directory").expect("create blocking file");

        let effects = state.handle_native_message(page_cache_capture(
            "READ",
            false,
            false,
            "<html>ok</html>",
        ));

        assert_cache_status_effect(&effects, false);
    }

    #[test]
    fn page_cache_status_reports_exact_cached_entry() {
        let (mut state, page_cache_path) = page_cache_test_state("status-hit");
        let _ = state.handle_native_message(page_cache_capture(
            "READ",
            false,
            false,
            "<html>ok</html>",
        ));
        assert!(cached_entry(&page_cache_path).is_some());

        let effects = state.handle_native_message(page_cache_status_request(
            42,
            "https://example.com/page#section",
        ));

        assert_cache_status_effect(&effects, true);
    }

    #[test]
    fn page_cache_status_does_not_report_canonical_match() {
        let (mut state, page_cache_path) = page_cache_test_state("status-canonical-miss");
        let _ = state.handle_native_message(page_cache_capture(
            "READ",
            false,
            false,
            "<html>ok</html>",
        ));
        assert!(cached_entry(&page_cache_path).is_some());

        let effects = state.handle_native_message(page_cache_status_request(
            42,
            "https://example.com/page#other",
        ));

        let [HostEffect::SendNative(message)] = &effects[..] else {
            panic!("expected one native status response");
        };
        assert_eq!(
            message.data.as_ref().expect("status data")["available"],
            false
        );
    }

    #[test]
    fn page_cache_capture_with_many_existing_files_has_fast_post_settle_contract() {
        let (mut state, page_cache_path) = page_cache_test_state("many-existing-fast-contract");
        let mut cache = PageCache::default();
        for i in 0..200 {
            let html = format!("<html>unrelated {i}</html>");
            cache.store_success(
                Some("other-profile"),
                i,
                &format!("https://unrelated.example/{i}"),
                Some("Unrelated"),
                &html,
                html.chars().count() as i64,
                i,
                Some("complete"),
                false,
                false,
                i,
            );
        }
        cache
            .save_if_dirty(&page_cache_path)
            .expect("seed cache files");

        let before_files: Vec<_> = fs::read_dir(&page_cache_path)
            .expect("read seeded cache")
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
            .map(|path| {
                let modified = fs::metadata(&path)
                    .expect("metadata before")
                    .modified()
                    .expect("modified before");
                (path, modified)
            })
            .collect();
        assert_eq!(before_files.len(), 200);
        std::thread::sleep(Duration::from_millis(20));

        let started = Instant::now();
        let effects = state.handle_native_message(page_cache_capture(
            "READ",
            false,
            false,
            "<html>ok</html>",
        ));
        let elapsed = started.elapsed();

        assert_cache_status_effect(&effects, true);
        let modified_existing = before_files
            .iter()
            .filter(|(path, before)| {
                fs::metadata(path)
                    .and_then(|metadata| metadata.modified())
                    .map(|after| after != *before)
                    .unwrap_or(true)
            })
            .count();
        let after_file_count = fs::read_dir(&page_cache_path)
            .expect("read cache after capture")
            .filter_map(Result::ok)
            .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("json"))
            .count();

        assert_eq!(
            modified_existing, 0,
            "post-settle capture should not rewrite unrelated cache files"
        );
        assert_eq!(after_file_count, before_files.len() + 1);
        assert!(
            elapsed < Duration::from_millis(300),
            "post-settle cache capture should complete under 0.3s, got {elapsed:?}"
        );
    }

    #[test]
    fn non_ping_browser_actions_fail_fast_without_native_channel() {
        let path = state_path("no-native");
        let mut state = HostState::new_with_native_channel(
            path.join("undo.jsonl"),
            path.join("focus.db"),
            None,
            false,
        );
        let effects = state.handle_cli_request(
            7,
            RequestEnvelope {
                id: Some("req-list".to_string()),
                action: "list".to_string(),
                params: Value::Object(Map::new()),
                auth_token: None,
            },
        );
        let HostEffect::Respond { client_id, payload } = &effects[0] else {
            panic!("expected immediate error response");
        };
        assert_eq!(*client_id, 7);
        assert!(!payload.ok);
        assert_eq!(payload.action.as_deref(), Some("list"));
        assert_eq!(payload.request_id.as_deref(), Some("req-list"));
        assert_eq!(
            payload.error.as_ref().map(|err| err.message.as_str()),
            Some("Native browser channel unavailable")
        );
    }

    #[test]
    fn collect_timed_out_requests_returns_error_without_native_message() {
        let mut state = test_state();
        let request = RequestEnvelope {
            id: Some("req-1".to_string()),
            action: "analyze".to_string(),
            params: Value::Object(Map::new()),
            auth_token: None,
        };

        let effects = state.handle_cli_request(7, request);
        let HostEffect::SendNative(native) = &effects[0] else {
            panic!("expected native forward");
        };
        let pending = state
            .pending
            .get_mut(&native.id)
            .expect("pending request should exist");
        pending.created_at = now_ms().saturating_sub(REQUEST_TIMEOUT_MS + 1);

        let effects = state.collect_timed_out_requests();
        assert_eq!(effects.len(), 1);
        let HostEffect::Respond { client_id, payload } = &effects[0] else {
            panic!("expected timeout response");
        };
        assert_eq!(*client_id, 7);
        assert!(!payload.ok);
        assert_eq!(payload.action.as_deref(), Some("analyze"));
        assert_eq!(payload.request_id.as_deref(), Some("req-1"));
        assert_eq!(
            payload.error.as_ref().map(|err| err.message.as_str()),
            Some("Request timed out")
        );
    }
}
