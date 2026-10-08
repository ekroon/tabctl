use serde_json::{json, Map, Value};
use std::path::Path;

use super::orchestrate::scope::select_tabs_by_scope;
use super::policy::affected_tab_ids;
use super::protocol::now_ms;
use super::undo::append_undo_record;

/// Write-ahead recovery stores only tabs targeted by issued primitives. Undo
/// reconciles their current existence, including an effect whose reply was lost.
#[derive(Debug)]
pub(super) struct Transaction {
    pub(super) snapshot: Value,
    pub(super) record: Map<String, Value>,
    pub(super) persisted: bool,
}

impl Transaction {
    pub(super) fn new(txid: &str, action: &str) -> Self {
        Self {
            snapshot: Value::Null,
            record: json!({
                "txid": txid, "action": action, "createdAt": now_ms(), "status": "pending",
                "undo": {"action": "restore", "tabs": [], "cleanupTabs": []},
            })
            .as_object()
            .unwrap()
            .clone(),
            persisted: false,
        }
    }

    pub(super) fn prepare(
        &mut self,
        action: &str,
        params: &Value,
        path: &Path,
    ) -> Result<(), String> {
        if self.snapshot.is_null() {
            return Err("Cannot mutate without a recovery snapshot".into());
        }
        let ids = affected_tab_ids(&self.snapshot, action, params);
        let tabs = select_tabs_by_scope(&self.snapshot, &json!({"tabIds": ids})).tabs;
        let target_incognito = params
            .get("windowId")
            .and_then(Value::as_i64)
            .is_some_and(|id| {
                self.snapshot["windows"].as_array().is_some_and(|windows| {
                    windows.iter().any(|window| {
                        window["windowId"].as_i64() == Some(id)
                            && window["incognito"].as_bool() == Some(true)
                    })
                })
            })
            || params["incognito"].as_bool() == Some(true);
        let undo = self.record.get_mut("undo").unwrap();
        if target_incognito || tabs.iter().any(|tab| tab.incognito) {
            // Never persist private data, nor advertise durable undo for it.
            if self.persisted {
                return Err("Cannot mix private tabs into a persisted transaction".into());
            }
            undo["incognito"] = json!(true);
        }
        for tab in tabs {
            let entries = undo["tabs"].as_array_mut().unwrap();
            if !entries
                .iter()
                .any(|entry| entry["tabId"].as_i64() == Some(tab.tab_id))
            {
                entries.push(json!({
                    "tabId": tab.tab_id, "url": tab.url, "title": tab.title,
                    "pinned": tab.pinned, "active": tab.active,
                    "createIfMissing": matches!(action, "p:tab-remove" | "p:window-remove"),
                    "from": {
                        "windowId": tab.window_id, "index": tab.index, "groupId": tab.group_id,
                        "groupTitle": tab.group_title, "groupColor": tab.group_color,
                        "groupCollapsed": tab.group_collapsed,
                    }
                }));
            } else if matches!(action, "p:tab-remove" | "p:window-remove") {
                let entry = entries
                    .iter_mut()
                    .find(|entry| entry["tabId"].as_i64() == Some(tab.tab_id))
                    .unwrap();
                entry["createIfMissing"] = json!(true);
            }
        }
        self.record.insert("status".into(), json!("in_progress"));
        self.record.insert(
            "inFlight".into(),
            json!({"action": action, "params": params}),
        );
        self.persist(path)
    }

    pub(super) fn acknowledge(
        &mut self,
        action: &str,
        response: &Value,
        path: &Path,
    ) -> Result<(), String> {
        if self.record["inFlight"]["action"].as_str() != Some(action) {
            return Err("Primitive reply does not match the issued recovery step".into());
        }
        if action == "p:window-create" {
            if response
                .get("id")
                .or_else(|| response.get("windowId"))
                .and_then(Value::as_i64)
                .is_none()
            {
                return Err(
                    "Creation reply did not establish the created window ID; recovery is uncertain"
                        .into(),
                );
            }
            let moved_tab = self.record["inFlight"]["params"]
                .get("tabId")
                .and_then(Value::as_i64);
            if moved_tab.is_none() {
                let tabs = response
                    .get("tabs")
                    .and_then(Value::as_array)
                    .filter(|tabs| !tabs.is_empty())
                    .ok_or(
                        "Creation reply did not establish created tab IDs; recovery is uncertain",
                    )?;
                let ids: Option<Vec<_>> = tabs
                    .iter()
                    .map(|tab| {
                        tab.get("id")
                            .or_else(|| tab.get("tabId"))
                            .and_then(Value::as_i64)
                    })
                    .collect();
                let ids =
                    ids.ok_or("Creation reply contains unknown tab IDs; recovery is uncertain")?;
                let cleanup = self.record.get_mut("undo").unwrap()["cleanupTabs"]
                    .as_array_mut()
                    .unwrap();
                cleanup.extend(ids.into_iter().map(|id| json!(id)));
            }
        }
        if action == "p:tab-create" {
            let id = response
                .get("id")
                .or_else(|| response.get("tabId"))
                .and_then(Value::as_i64)
                .ok_or(
                    "Creation reply did not establish the created tab ID; recovery is uncertain",
                )?;
            self.record.get_mut("undo").unwrap()["cleanupTabs"]
                .as_array_mut()
                .unwrap()
                .push(json!(id));
        }
        self.record.remove("inFlight");
        self.persist(path)
    }

    pub(super) fn finish(
        &mut self,
        status: &str,
        summary: Value,
        path: &Path,
    ) -> Result<(), String> {
        self.record.insert("status".into(), json!(status));
        self.record.insert("summary".into(), summary);
        self.persist(path)
    }

    fn persist(&mut self, path: &Path) -> Result<(), String> {
        self.persisted = append_undo_record(path, &self.record)
            .map_err(|err| format!("Cannot persist undo recovery: {err}"))?;
        Ok(())
    }
}

pub(super) fn is_mutating_primitive(action: &str) -> bool {
    matches!(
        action,
        "p:tab-create"
            | "p:tab-remove"
            | "p:tab-move"
            | "p:tab-group"
            | "p:tab-ungroup"
            | "p:group-update"
            | "p:group-move"
            | "p:window-create"
            | "p:window-remove"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_contains_issued_targets_not_unexecuted_steps() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/transaction-tests")
            .join(super::super::protocol::create_id("test"))
            .join("undo.jsonl");
        let mut tx = Transaction::new("tx-1", "close");
        tx.snapshot = json!({"windows":[{"windowId":1,"tabs":[
            {"tabId":1,"url":"https://a.example","index":0},
            {"tabId":2,"url":"https://b.example","index":1}
        ]}]});
        tx.prepare("p:tab-remove", &json!({"tabIds":[1]}), &path)
            .unwrap();
        let records = super::super::undo::read_undo_records(&path);
        assert_eq!(records[0]["undo"]["tabs"].as_array().unwrap().len(), 1);
        assert_eq!(records[0]["undo"]["tabs"][0]["tabId"], 1);
        assert_eq!(records[0]["inFlight"]["action"], "p:tab-remove");
        tx.acknowledge("p:tab-remove", &json!({}), &path).unwrap();
        tx.prepare(
            "p:tab-create",
            &json!({"windowId":1,"url":"https://new.example"}),
            &path,
        )
        .unwrap();
        tx.acknowledge("p:tab-create", &json!({"id":9}), &path)
            .unwrap();
        assert_eq!(tx.record["undo"]["cleanupTabs"], json!([9]));
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn creation_metadata_and_reply_identity_must_be_known_before_clearing_in_flight() {
        for (action, response) in [
            ("p:tab-create", json!({})),
            ("p:window-create", json!({"id":99})),
            ("p:window-create", json!({"id":99,"tabs":[{"id":8},{}]})),
        ] {
            let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/transaction-tests")
                .join(super::super::protocol::create_id("test"))
                .join("undo.jsonl");
            let mut tx = Transaction::new("creation", "open");
            tx.snapshot = json!({"windows":[]});
            tx.prepare(action, &json!({}), &path).unwrap();
            let original = tx.record.clone();
            assert!(tx
                .acknowledge(action, &response, &path)
                .unwrap_err()
                .contains("uncertain"));
            assert_eq!(tx.record, original);
            assert!(tx.acknowledge("p:tab-remove", &json!({}), &path).is_err());
            assert_eq!(tx.record, original);
            std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
        }
    }
}
