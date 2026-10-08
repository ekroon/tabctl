use serde_json::Value;
use std::time::Duration;
use tabctl_shared::{NativeMessage, ProtocolError};

use super::{AnalysisRecord, HostEffect, HostState, PendingRequest};
use crate::host_impl::orchestrate::{OrchStep, Orchestration};
use crate::host_impl::protocol::{base_response, create_id, now_ms, trace_line, value_object};
use crate::host_impl::transaction::is_mutating_primitive;
use crate::host_impl::undo::{append_undo_record, find_undo_record};

impl HostState {
    pub(in crate::host_impl) fn process_orch_step(
        &mut self,
        client_id: u64,
        action: &str,
        request_id: Option<String>,
        txid: Option<String>,
        step: OrchStep,
        mut orch: Box<dyn Orchestration>,
    ) -> Vec<HostEffect> {
        if action == "undo"
            && txid
                .as_ref()
                .is_some_and(|id| self.transactions.contains_key(id))
        {
            return vec![self.error_effect(
                PendingRequest {
                    client_id,
                    action: action.into(),
                    request_id,
                    txid,
                    created_at: now_ms(),
                    orchestration: Some(orch),
                    primitive: None,
                },
                create_id("orch"),
                "Cannot undo an active transaction".into(),
                None,
            )];
        }
        match step {
            OrchStep::SendPrimitive {
                action: prim_action,
                params,
            } => {
                if is_mutating_primitive(&prim_action)
                    || (action == "undo" && prim_action == "p:tab-update")
                {
                    let preparation = if action == "undo" {
                        txid.as_ref()
                            .ok_or_else(|| "Undo mutation has no recovery transaction".to_string())
                            .and_then(|id| find_undo_record(&self.undo_log, id))
                            .and_then(|record| match record {
                                Some(record)
                                    if record.get("status").and_then(Value::as_str)
                                        == Some("undoing") =>
                                {
                                    Ok(())
                                }
                                Some(_) => Err("Undo transaction is no longer in progress".into()),
                                None => {
                                    Err("Undo record disappeared before mutation dispatch".into())
                                }
                            })
                    } else {
                        txid.as_ref()
                            .and_then(|id| self.transactions.get_mut(id))
                            .map(|tx| {
                                self.policy
                                    .check_primitive(&tx.snapshot, &prim_action, &params)?;
                                tx.prepare(&prim_action, &params, &self.undo_log)
                            })
                            .unwrap_or_else(|| Err("Mutation has no recovery transaction".into()))
                    };
                    if let Err(message) = preparation {
                        return vec![self.error_effect(
                            PendingRequest {
                                client_id,
                                action: action.into(),
                                request_id,
                                txid,
                                created_at: now_ms(),
                                orchestration: Some(orch),
                                primitive: None,
                            },
                            create_id("orch"),
                            message,
                            None,
                        )];
                    }
                }
                let new_id = create_id("orch");
                self.pending.insert(
                    new_id.clone(),
                    PendingRequest {
                        client_id,
                        action: action.into(),
                        request_id,
                        txid,
                        created_at: now_ms(),
                        orchestration: Some(orch),
                        primitive: Some(prim_action.clone()),
                    },
                );
                trace_line(&format!("pending orch step: client_id={client_id} request_id={new_id} action={action} native_action={prim_action}"));
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
            OrchStep::Delay { duration_ms } => {
                std::thread::sleep(Duration::from_millis(duration_ms));
                let next = orch.step(Value::Null);
                self.process_orch_step(client_id, action, request_id, txid, next, orch)
            }
            OrchStep::Complete { response, undo } => {
                let mut durable_txid = None;
                let mut private_mutation = false;
                if let Some(id) = txid.as_ref() {
                    if action == "undo" {
                        let checkpoint = find_undo_record(&self.undo_log, id).and_then(|record| {
                            let mut record = record.ok_or_else(|| {
                                "Undo record disappeared before completion".to_string()
                            })?;
                            record.insert("status".into(), Value::String("undone".into()));
                            record.insert("undoneAt".into(), Value::Number(now_ms().into()));
                            append_undo_record(&self.undo_log, &record)
                        });
                        if let Err(message) = checkpoint {
                            let mut resp = base_response(false, Some(action.into()), request_id);
                            resp.error = Some(ProtocolError {
                                    message: format!("Restoration applied but cannot mark transaction undone: {message}"),
                                    hint: Some("Do not replay this transaction; inspect browser state first.".into()),
                                });
                            resp.data = Some(
                                serde_json::json!({"txid":id,"status":"undo_checkpoint_failed"}),
                            );
                            return vec![HostEffect::Respond {
                                client_id,
                                payload: resp,
                            }];
                        }
                        durable_txid = Some(id.clone());
                    } else if let Some(tx) = self.transactions.get_mut(id) {
                        private_mutation = tx.record["undo"]["incognito"].as_bool() == Some(true);
                        let summary = response
                            .get(if action == "analyze" {
                                "dedupeSummary"
                            } else {
                                "summary"
                            })
                            .cloned()
                            .unwrap_or_else(|| serde_json::json!({}));
                        if tx.persisted || private_mutation {
                            if let Err(message) = tx.finish("completed", summary, &self.undo_log) {
                                return vec![self.error_effect(
                                    PendingRequest {
                                        client_id,
                                        action: action.into(),
                                        request_id,
                                        txid,
                                        created_at: now_ms(),
                                        orchestration: Some(orch),
                                        primitive: None,
                                    },
                                    create_id("orch"),
                                    message,
                                    None,
                                )];
                            }
                            if tx.persisted {
                                durable_txid = Some(id.clone());
                            }
                        }
                        self.transactions.remove(id);
                    }
                }
                let mut resp = base_response(true, Some(action.into()), request_id);
                let mut data = value_object(Some(response));
                if private_mutation {
                    data.insert(
                        "undoUnavailable".into(),
                        Value::String("private tabs are never persisted".into()),
                    );
                }
                if action == "analyze" {
                    let analysis_id = create_id("analysis");
                    self.analyses
                        .insert(analysis_id.clone(), AnalysisRecord { data: data.clone() });
                    data.insert("analysisId".into(), Value::String(analysis_id));
                }
                if txid.is_some() || undo.is_some() || action == "close" {
                    data.insert(
                        "txid".into(),
                        durable_txid.map(Value::String).unwrap_or(Value::Null),
                    );
                }
                resp.data = Some(Value::Object(data));
                vec![HostEffect::Respond {
                    client_id,
                    payload: resp,
                }]
            }
            OrchStep::Error { message, hint } => vec![self.error_effect(
                PendingRequest {
                    client_id,
                    action: action.into(),
                    request_id,
                    txid,
                    created_at: now_ms(),
                    orchestration: Some(orch),
                    primitive: None,
                },
                create_id("orch"),
                message,
                hint,
            )],
            OrchStep::Progress { data } => {
                let mut resp = base_response(true, Some(action.into()), request_id.clone());
                resp.progress = Some(true);
                resp.data = Some(data);
                let mut effects = vec![HostEffect::Respond {
                    client_id,
                    payload: resp,
                }];
                let next = orch.step(Value::Null);
                effects.extend(
                    self.process_orch_step(client_id, action, request_id, txid, next, orch),
                );
                effects
            }
        }
    }
}
