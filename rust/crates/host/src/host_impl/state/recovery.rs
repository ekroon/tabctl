use serde_json::Value;
use tabctl_shared::ProtocolError;

use super::{HostEffect, HostState, PendingRequest};
use crate::host_impl::protocol::base_response;
use crate::host_impl::undo::{append_undo_record, find_undo_record, unresolved_creation};

impl HostState {
    pub(super) fn error_effect(
        &mut self,
        pending: PendingRequest,
        message_id: String,
        mut message: String,
        mut hint: Option<String>,
    ) -> HostEffect {
        let resp_id = pending.request_id.clone().unwrap_or(message_id);
        let mut recovery_txid = None;
        let mut status = "failed";
        if let Some(txid) = pending.txid.as_ref() {
            if pending.action == "undo" {
                // Undo replies never own the original transaction's live state.
                if !self.transactions.contains_key(txid) {
                    match find_undo_record(&self.undo_log, txid) {
                        Ok(Some(mut record)) => {
                            let uncertain = matches!(
                                pending.primitive.as_deref(),
                                Some("p:tab-create" | "p:window-create")
                            );
                            status = if uncertain {
                                "undo_uncertain"
                            } else {
                                "undo_failed"
                            };
                            recovery_txid = Some(txid.clone());
                            record.insert("status".into(), Value::String(status.into()));
                            if uncertain {
                                hint = Some("Creation effects and IDs are unknown. Preserve the journal and inspect browser state manually; automatic replay is blocked.".into());
                            }
                            if let Err(error) = append_undo_record(&self.undo_log, &record) {
                                message =
                                    format!("{message}; cannot checkpoint failed undo: {error}");
                            }
                        }
                        Ok(None) => {
                            message = format!(
                                "{message}; undo record disappeared before failure checkpoint"
                            )
                        }
                        Err(error) => message = format!("{message}; {error}"),
                    }
                }
            } else if let Some(mut tx) = self.transactions.remove(txid) {
                let uncertain = unresolved_creation(&tx.record);
                status = if uncertain {
                    "recovery_uncertain"
                } else {
                    "failed"
                };
                if tx.persisted {
                    recovery_txid = Some(txid.clone());
                    let recovery_hint = if uncertain {
                        format!("Transaction {txid} has unresolved creation effects and IDs. Preserve the journal and inspect browser state manually; automatic undo is blocked.")
                    } else {
                        format!("Recovery transaction {txid} is available in history; inspect browser state and undo this transaction to recover partial effects.")
                    };
                    hint = Some(
                        hint.map(|hint| format!("{hint} {recovery_hint}"))
                            .unwrap_or(recovery_hint),
                    );
                }
                if tx.persisted || tx.record.contains_key("inFlight") {
                    if let Err(error) = tx.finish(status, serde_json::json!({}), &self.undo_log) {
                        message = format!("{message}; {error}");
                    }
                }
            }
        }
        let mut resp = base_response(false, Some(pending.action), Some(resp_id));
        resp.error = Some(ProtocolError { message, hint });
        if recovery_txid.is_some() {
            resp.data = Some(serde_json::json!({"txid":recovery_txid,"status":status}));
        }
        HostEffect::Respond {
            client_id: pending.client_id,
            payload: resp,
        }
    }

    pub(super) fn timeout_effect(
        &mut self,
        pending: PendingRequest,
        message_id: String,
    ) -> HostEffect {
        self.error_effect(pending, message_id, "Request timed out".into(), None)
    }
}
