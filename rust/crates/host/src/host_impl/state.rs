use serde_json::{Map, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use tabctl_shared::{ClientInfo, NativeMessage, ProtocolError, RequestEnvelope, ResponseEnvelope};

use super::browser_state;
use super::focus_store;
use super::orchestrate::{orchestration_for, OrchStep, Orchestration};
use super::protocol::{
    add_host_metadata, add_ping_metadata, base_response, create_id, host_ping_data, host_version,
    local_actions, log_line, now_ms, trace_line, undo_actions, value_object, version_info_value,
    REQUEST_TIMEOUT_MS,
};
use super::undo::{
    append_undo_record, filter_by_retention, find_latest_undo_record, find_undo_record,
    read_undo_records, RETENTION_DAYS,
};

const HISTORY_LIMIT_DEFAULT: usize = 20;

#[derive(Debug)]
struct PendingRequest {
    client_id: u64,
    action: String,
    request_id: Option<String>,
    txid: Option<String>,
    created_at: u64,
    orchestration: Option<Box<dyn Orchestration>>,
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
    profile_name: Option<String>,
    native_channel_available: bool,
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
    fn error_effect(
        pending: PendingRequest,
        message_id: String,
        message: String,
        hint: Option<String>,
    ) -> HostEffect {
        let resp_id = pending.request_id.clone().unwrap_or(message_id);
        let mut resp = base_response(false, Some(pending.action), Some(resp_id));
        resp.error = Some(ProtocolError { message, hint });
        HostEffect::Respond {
            client_id: pending.client_id,
            payload: resp,
        }
    }

    fn timeout_effect(pending: PendingRequest, message_id: String) -> HostEffect {
        Self::error_effect(pending, message_id, "Request timed out".to_string(), None)
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
        let state_db_path = undo_log
            .parent()
            .map(|parent| parent.join("state.db"))
            .unwrap_or_else(|| PathBuf::from("state.db"));
        Self {
            pending: HashMap::new(),
            analyses: HashMap::new(),
            undo_log,
            state_db_path,
            focus_db_path,
            profile_name,
            native_channel_available,
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

        expired
            .into_iter()
            .filter_map(|message_id| {
                self.pending
                    .remove(&message_id)
                    .map(|pending| Self::timeout_effect(pending, message_id))
            })
            .collect()
    }

    pub(super) fn fail_pending_request(
        &mut self,
        message_id: &str,
        message: String,
        hint: Option<String>,
    ) -> Option<HostEffect> {
        self.pending
            .remove(message_id)
            .map(|pending| Self::error_effect(pending, message_id.to_string(), message, hint))
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

        if action == "ping" {
            let mut resp = base_response(true, Some(action), request.id);
            add_host_metadata(&mut resp);
            resp.data = Some(Value::Object(host_ping_data(self.native_channel_available)));
            return vec![HostEffect::Respond {
                client_id,
                payload: resp,
            }];
        }

        if !self.native_channel_available && !local_actions().contains(action.as_str()) {
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

            let undo_params = Value::Object(Map::from_iter([(
                "record".to_string(),
                Value::Object(record),
            )]));
            if let Some(mut orch) = orchestration_for("undo", &undo_params) {
                let step = orch.start();
                return self.process_orch_step(client_id, "undo", request.id, None, step, orch);
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
        if let Some(mut orch) = orchestration_for(&action, &request.params) {
            let txid = if undo_actions().contains(action.as_str()) {
                Some(create_id("tx"))
            } else {
                None
            };
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
                return vec![Self::timeout_effect(timed_out, message_id)];
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

        // Extension error — abort orchestration if active
        if !message.ok.unwrap_or(false) {
            let resp_id = pending.request_id.clone().unwrap_or(message_id);
            let mut resp = base_response(false, Some(pending.action), Some(resp_id));
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
            }
            let step = orch.step(response_data);
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
        // Preserve original data shape (arrays, objects, etc.)
        resp.data = Some(message.data.unwrap_or(Value::Object(message_data)));
        vec![HostEffect::Respond {
            client_id: pending.client_id,
            payload: resp,
        }]
    }

    /// Process an orchestration step result: send the next primitive, complete,
    /// or return an error.
    pub(super) fn process_orch_step(
        &mut self,
        client_id: u64,
        action: &str,
        request_id: Option<String>,
        txid: Option<String>,
        step: OrchStep,
        mut orch: Box<dyn Orchestration>,
    ) -> Vec<HostEffect> {
        match step {
            OrchStep::SendPrimitive {
                action: prim_action,
                params,
            } => {
                let new_id = create_id("orch");
                self.pending.insert(
                    new_id.clone(),
                    PendingRequest {
                        client_id,
                        action: action.to_string(),
                        request_id: request_id.clone(),
                        txid,
                        created_at: now_ms(),
                        orchestration: Some(orch),
                    },
                );
                trace_line(&format!(
                    "pending orch step: client_id={} request_id={} action={} native_action={}",
                    client_id, new_id, action, prim_action
                ));
                vec![HostEffect::SendNative(NativeMessage {
                    id: new_id,
                    action: Some(prim_action),
                    ok: None,
                    progress: None,
                    params: Some(params),
                    data: None,
                    error: None,
                })]
            }
            OrchStep::Complete { response, undo } => {
                if let Some(ref undo_data) = undo {
                    if let Some(ref txid_str) = txid {
                        let summary = response
                            .as_object()
                            .and_then(|o| o.get("summary"))
                            .cloned()
                            .unwrap_or_else(|| Value::Object(Map::new()));
                        let mut record = Map::new();
                        record.insert("txid".to_string(), Value::String(txid_str.clone()));
                        record.insert("createdAt".to_string(), Value::Number(now_ms().into()));
                        record.insert("action".to_string(), Value::String(action.to_string()));
                        record.insert("summary".to_string(), summary);
                        record.insert("undo".to_string(), undo_data.clone());
                        append_undo_record(&self.undo_log, &record);
                    }
                }
                let mut resp = base_response(true, Some(action.to_string()), request_id);
                let mut data = value_object(Some(response));

                // Cache analysis for close --apply
                if action == "analyze" {
                    let analysis_id = create_id("analysis");
                    self.analyses
                        .insert(analysis_id.clone(), AnalysisRecord { data: data.clone() });
                    data.insert("analysisId".to_string(), Value::String(analysis_id));
                }

                if let Some(txid_str) = txid {
                    data.insert("txid".to_string(), Value::String(txid_str));
                }
                resp.data = Some(Value::Object(data));
                vec![HostEffect::Respond {
                    client_id,
                    payload: resp,
                }]
            }
            OrchStep::Error { message, hint } => {
                let mut resp = base_response(false, Some(action.to_string()), request_id);
                resp.error = Some(ProtocolError { message, hint });
                vec![HostEffect::Respond {
                    client_id,
                    payload: resp,
                }]
            }
            OrchStep::Progress { data } => {
                let mut resp = base_response(true, Some(action.to_string()), request_id.clone());
                resp.progress = Some(true);
                resp.data = Some(data);
                let mut effects = vec![HostEffect::Respond {
                    client_id,
                    payload: resp,
                }];
                let next_step = orch.step(Value::Null);
                effects.extend(
                    self.process_orch_step(client_id, action, request_id, txid, next_step, orch),
                );
                effects
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Map, Value};

    fn test_state() -> HostState {
        HostState::new(
            std::env::temp_dir().join("tabctl-host-state-test-undo.jsonl"),
            std::env::temp_dir().join("tabctl-host-state-test-focus.db"),
            None,
        )
    }

    #[test]
    fn non_ping_browser_actions_fail_fast_without_native_channel() {
        let mut state = HostState::new_with_native_channel(
            std::env::temp_dir().join("tabctl-host-state-test-undo.jsonl"),
            std::env::temp_dir().join("tabctl-host-state-test-focus.db"),
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
