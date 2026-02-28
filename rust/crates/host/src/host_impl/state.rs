use serde_json::{Map, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use tabctl_shared::{ClientInfo, NativeMessage, ProtocolError, RequestEnvelope, ResponseEnvelope};

use super::orchestrate::{orchestration_for, OrchStep, Orchestration};
use super::protocol::{
    add_host_metadata, add_ping_metadata, base_response, create_id, host_version, local_actions,
    now_ms, undo_actions, value_object, version_info_value, REQUEST_TIMEOUT_MS,
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
    pub(super) fn new(undo_log: PathBuf) -> Self {
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
        self.forward_to_extension_with_orch(client_id, request, txid, None)
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
                request_id: request.id.clone(),
                txid,
                created_at: now_ms(),
                orchestration,
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

        // Timeout check
        if let Some(pending) = self.pending.get(&message_id) {
            if now_ms().saturating_sub(pending.created_at) > REQUEST_TIMEOUT_MS {
                let timed_out = self.pending.remove(&message_id).expect("pending exists");
                let resp_id = timed_out.request_id.clone().unwrap_or(message_id);
                let mut resp = base_response(false, Some(timed_out.action), Some(resp_id));
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
            // Pass original data shape (may be array, object, or null)
            let step = orch.step(message.data.clone().unwrap_or(Value::Object(message_data)));
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
