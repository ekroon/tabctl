use serde_json::{Map, Value};

use super::scope::{select_tabs_by_scope, ScopedTab};
use super::OrchStep;

/// Orchestration for the `close` command.
///
/// Two steps: p:snapshot → select tabs by scope → p:tab-remove.
/// Produces an undo record with full tab metadata for restoration.
#[derive(Debug)]
pub(crate) struct CloseOrchestration {
    params: Value,
    phase: ClosePhase,
    close_state: Option<CloseState>,
}

#[derive(Debug)]
struct CloseState {
    valid_tabs: Vec<ClosedTabInfo>,
    skipped: Vec<Value>,
}

#[derive(Debug)]
struct ClosedTabInfo {
    url: Option<String>,
    title: Option<String>,
    pinned: Option<bool>,
    active: Option<bool>,
    window_id: i64,
    index: Option<i64>,
    group_id: i64,
    group_title: Option<String>,
    group_color: Option<String>,
    group_collapsed: Option<bool>,
}

#[derive(Debug)]
enum ClosePhase {
    GetSnapshot,
    RemoveTabs,
}

impl CloseOrchestration {
    pub(crate) fn new(params: &Value) -> Self {
        Self {
            params: params.clone(),
            phase: ClosePhase::GetSnapshot,
            close_state: None,
        }
    }
}

impl super::Orchestration for CloseOrchestration {
    fn start(&mut self) -> OrchStep {
        // Validation: direct mode requires confirmation
        let mode = self
            .params
            .get("mode")
            .and_then(Value::as_str)
            .unwrap_or("direct");
        if mode == "direct" && self.params.get("confirmed").and_then(Value::as_bool) != Some(true) {
            return OrchStep::Error {
                message: "Direct close requires confirmation".to_string(),
                hint: None,
            };
        }

        OrchStep::SendPrimitive {
            action: "p:snapshot".to_string(),
            params: Value::Object(Map::new()),
        }
    }

    fn step(&mut self, response: Value) -> OrchStep {
        match self.phase {
            ClosePhase::GetSnapshot => {
                // Get tab IDs from params or scope selection
                let mut tab_ids: Vec<i64> = self
                    .params
                    .get("tabIds")
                    .and_then(Value::as_array)
                    .map(|arr| arr.iter().filter_map(Value::as_i64).collect())
                    .unwrap_or_default();

                if tab_ids.is_empty() {
                    let scope_result = select_tabs_by_scope(&response, &self.params);
                    if let Some(err) = scope_result.error {
                        return OrchStep::Error {
                            message: err,
                            hint: None,
                        };
                    }
                    tab_ids = scope_result.tabs.iter().map(|t| t.tab_id).collect();
                }

                if tab_ids.is_empty() {
                    return OrchStep::Complete {
                        response: serde_json::json!({
                            "summary": { "closedTabs": 0, "skippedTabs": 0 },
                            "skipped": [],
                        }),
                        undo: Some(serde_json::json!({
                            "action": "close",
                            "tabs": [],
                        })),
                    };
                }

                // Build tab info from snapshot for undo records
                let all_tabs = flatten_snapshot_tabs(&response);
                let expected_urls: Map<String, Value> = self
                    .params
                    .get("expectedUrls")
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default();

                // Build group index from snapshot
                let group_index = build_group_index(&response);

                let mut valid_tabs = Vec::new();
                let mut skipped = Vec::new();
                let mut remove_ids = Vec::new();

                for tab_id in &tab_ids {
                    let Some(tab) = all_tabs.iter().find(|t| t.tab_id == *tab_id) else {
                        skipped.push(serde_json::json!({"tabId": tab_id, "reason": "not_found"}));
                        continue;
                    };

                    // URL mismatch check (for apply mode)
                    if let Some(expected) = expected_urls
                        .get(&tab_id.to_string())
                        .and_then(Value::as_str)
                    {
                        if tab.url.as_deref() != Some(expected) {
                            skipped.push(
                                serde_json::json!({"tabId": tab_id, "reason": "url_mismatch"}),
                            );
                            continue;
                        }
                    }

                    // Get group info
                    let (group_title, group_color, group_collapsed) = if tab.group_id != -1 {
                        group_index
                            .get(&tab.group_id)
                            .map(|g| (g.0.clone(), g.1.clone(), g.2))
                            .unwrap_or((
                                tab.group_title.clone(),
                                tab.group_color.clone(),
                                tab.group_collapsed,
                            ))
                    } else {
                        (None, None, None)
                    };

                    valid_tabs.push(ClosedTabInfo {
                        url: tab.url.clone(),
                        title: tab.title.clone(),
                        pinned: tab.pinned,
                        active: tab.active,
                        window_id: tab.window_id,
                        index: tab.index,
                        group_id: tab.group_id,
                        group_title,
                        group_color,
                        group_collapsed,
                    });
                    remove_ids.push(*tab_id);
                }

                if remove_ids.is_empty() {
                    return OrchStep::Complete {
                        response: serde_json::json!({
                            "summary": {
                                "closedTabs": 0,
                                "skippedTabs": skipped.len(),
                            },
                            "skipped": skipped,
                        }),
                        undo: Some(serde_json::json!({
                            "action": "close",
                            "tabs": [],
                        })),
                    };
                }

                self.close_state = Some(CloseState {
                    valid_tabs,
                    skipped,
                });
                self.phase = ClosePhase::RemoveTabs;

                OrchStep::SendPrimitive {
                    action: "p:tab-remove".to_string(),
                    params: serde_json::json!({ "tabIds": remove_ids }),
                }
            }
            ClosePhase::RemoveTabs => {
                let state = self.close_state.as_ref().unwrap();

                let undo_tabs: Vec<Value> = state
                    .valid_tabs
                    .iter()
                    .map(|t| {
                        serde_json::json!({
                            "url": t.url,
                            "title": t.title,
                            "pinned": t.pinned,
                            "active": t.active,
                            "from": {
                                "windowId": t.window_id,
                                "index": t.index,
                                "groupId": t.group_id,
                                "groupTitle": t.group_title,
                                "groupColor": t.group_color,
                                "groupCollapsed": t.group_collapsed,
                            }
                        })
                    })
                    .collect();

                OrchStep::Complete {
                    response: serde_json::json!({
                        "summary": {
                            "closedTabs": state.valid_tabs.len(),
                            "skippedTabs": state.skipped.len(),
                        },
                        "skipped": state.skipped,
                    }),
                    undo: Some(serde_json::json!({
                        "action": "close",
                        "tabs": undo_tabs,
                    })),
                }
            }
        }
    }
}

type GroupInfo = (Option<String>, Option<String>, Option<bool>);

/// Build a group index from snapshot: groupId → (title, color, collapsed).
fn build_group_index(snapshot: &Value) -> std::collections::HashMap<i64, GroupInfo> {
    let mut idx = std::collections::HashMap::new();
    if let Some(windows) = snapshot.get("windows").and_then(Value::as_array) {
        for win in windows {
            if let Some(groups) = win.get("groups").and_then(Value::as_array) {
                for g in groups {
                    if let Some(gid) = g.get("groupId").and_then(Value::as_i64) {
                        idx.insert(
                            gid,
                            (
                                g.get("title").and_then(Value::as_str).map(String::from),
                                g.get("color").and_then(Value::as_str).map(String::from),
                                g.get("collapsed").and_then(Value::as_bool),
                            ),
                        );
                    }
                }
            }
        }
    }
    idx
}

/// Flatten tabs from snapshot with window context.
fn flatten_snapshot_tabs(snapshot: &Value) -> Vec<ScopedTab> {
    super::scope::select_tabs_by_scope(snapshot, &serde_json::json!({"all": true})).tabs
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host_impl::orchestrate::Orchestration;

    fn snapshot() -> Value {
        serde_json::json!({
            "windows": [{
                "windowId": 100,
                "focused": true,
                "tabs": [
                    {"tabId": 1, "windowId": 100, "index": 0, "url": "https://a.com", "title": "A", "active": true, "pinned": false, "groupId": 10, "groupTitle": "Dev", "groupColor": "blue", "groupCollapsed": false},
                    {"tabId": 2, "windowId": 100, "index": 1, "url": "https://b.com", "title": "B", "active": false, "pinned": true, "groupId": -1}
                ],
                "groups": [
                    {"groupId": 10, "title": "Dev", "color": "blue", "collapsed": false}
                ]
            }]
        })
    }

    #[test]
    fn close_by_tab_ids_with_undo() {
        let params = serde_json::json!({"tabIds": [1], "confirmed": true});
        let mut orch = CloseOrchestration::new(&params);

        let step = orch.start();
        assert!(matches!(&step, OrchStep::SendPrimitive { action, .. } if action == "p:snapshot"));

        let step = orch.step(snapshot());
        let OrchStep::SendPrimitive { action, params } = &step else {
            panic!("expected SendPrimitive, got {step:?}");
        };
        assert_eq!(action, "p:tab-remove");
        assert_eq!(params["tabIds"], serde_json::json!([1]));

        let step = orch.step(serde_json::json!({"removed": true}));
        let OrchStep::Complete { response, undo } = step else {
            panic!("expected Complete");
        };
        assert_eq!(response["summary"]["closedTabs"], 1);

        let undo = undo.unwrap();
        assert_eq!(undo["action"], "close");
        let tabs = undo["tabs"].as_array().unwrap();
        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs[0]["url"], "https://a.com");
        assert_eq!(tabs[0]["from"]["groupTitle"], "Dev");
    }

    #[test]
    fn close_requires_confirmation() {
        let params = serde_json::json!({"tabIds": [1]});
        let mut orch = CloseOrchestration::new(&params);
        let step = orch.start();
        assert!(
            matches!(&step, OrchStep::Error { message, .. } if message.contains("confirmation"))
        );
    }

    #[test]
    fn close_url_mismatch_skips_tab() {
        let params = serde_json::json!({
            "tabIds": [1],
            "confirmed": true,
            "expectedUrls": {"1": "https://wrong.com"}
        });
        let mut orch = CloseOrchestration::new(&params);
        let _ = orch.start();
        let step = orch.step(snapshot());

        let OrchStep::Complete { response, .. } = step else {
            panic!("expected Complete with no tabs to remove");
        };
        assert_eq!(response["summary"]["closedTabs"], 0);
        assert_eq!(response["summary"]["skippedTabs"], 1);
    }

    #[test]
    fn close_empty_tabs_returns_zero() {
        let params = serde_json::json!({"tabIds": [], "confirmed": true});
        let mut orch = CloseOrchestration::new(&params);
        let _ = orch.start();
        let step = orch.step(snapshot());
        // With empty tabIds and no scope, falls back to focused window (2 tabs)
        assert!(
            matches!(&step, OrchStep::SendPrimitive { action, .. } if action == "p:tab-remove")
        );
    }
}
