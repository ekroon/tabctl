use serde_json::{Map, Value};

use super::OrchStep;

/// Orchestration for the `list` command.
///
/// Single step: request p:snapshot, shape the response by projecting tab/group
/// fields (drops index, pinned, lastFocusedAt, state, groupCollapsed, groupColor).
#[derive(Debug)]
pub(crate) struct ListOrchestration;

impl ListOrchestration {
    pub(crate) fn new(_params: &Value) -> Self {
        Self
    }
}

impl super::Orchestration for ListOrchestration {
    fn start(&mut self) -> OrchStep {
        OrchStep::SendPrimitive {
            action: "p:snapshot".to_string(),
            params: Value::Object(Map::new()),
        }
    }

    fn step(&mut self, response: Value) -> OrchStep {
        let Some(windows) = response.get("windows").and_then(Value::as_array) else {
            return OrchStep::Error {
                message: "Snapshot missing windows".to_string(),
                hint: None,
            };
        };

        let shaped: Vec<Value> = windows
            .iter()
            .map(|win| {
                let tabs = win
                    .get("tabs")
                    .and_then(Value::as_array)
                    .map(|tabs| {
                        tabs.iter()
                            .map(|tab| {
                                serde_json::json!({
                                    "tabId": tab.get("tabId"),
                                    "windowId": tab.get("windowId"),
                                    "url": tab.get("url"),
                                    "title": tab.get("title"),
                                    "active": tab.get("active"),
                                    "groupId": tab.get("groupId"),
                                    "groupTitle": tab.get("groupTitle"),
                                })
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();

                let groups = win
                    .get("groups")
                    .and_then(Value::as_array)
                    .map(|groups| {
                        groups
                            .iter()
                            .map(|g| {
                                serde_json::json!({
                                    "groupId": g.get("groupId"),
                                    "title": g.get("title"),
                                    "color": g.get("color"),
                                    "collapsed": g.get("collapsed"),
                                })
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();

                serde_json::json!({
                    "windowId": win.get("windowId"),
                    "focused": win.get("focused"),
                    "tabs": tabs,
                    "groups": groups,
                })
            })
            .collect();

        OrchStep::Complete {
            response: serde_json::json!({ "windows": shaped }),
            undo: None,
        }
    }
}

/// Orchestration for the `group-list` command.
///
/// Single step: request p:snapshot, build a groups array with window labels
/// and tab counts.
#[derive(Debug)]
pub(crate) struct GroupListOrchestration {
    window_id_param: Option<Value>,
}

impl GroupListOrchestration {
    pub(crate) fn new(params: &Value) -> Self {
        Self {
            window_id_param: params.get("windowId").cloned(),
        }
    }
}

impl super::Orchestration for GroupListOrchestration {
    fn start(&mut self) -> OrchStep {
        OrchStep::SendPrimitive {
            action: "p:snapshot".to_string(),
            params: Value::Object(Map::new()),
        }
    }

    fn step(&mut self, response: Value) -> OrchStep {
        let Some(windows) = response.get("windows").and_then(Value::as_array) else {
            return OrchStep::Error {
                message: "Snapshot missing windows".to_string(),
                hint: None,
            };
        };

        // Resolve windowId filter
        let filter_window_id = self.window_id_param.as_ref().and_then(|v| {
            if let Some(n) = v.as_i64() {
                return Some(n);
            }
            if let Some(s) = v.as_str() {
                let normalized = s.trim().to_lowercase();
                if normalized == "active" || normalized == "last-focused" {
                    return windows.iter().find_map(|w| {
                        if w.get("focused").and_then(Value::as_bool) == Some(true) {
                            w.get("windowId").and_then(Value::as_i64)
                        } else {
                            None
                        }
                    });
                }
                return s.trim().parse::<i64>().ok();
            }
            None
        });

        let mut groups_out: Vec<Value> = Vec::new();

        for (idx, win) in windows.iter().enumerate() {
            let win_id = win.get("windowId").and_then(Value::as_i64).unwrap_or(0);

            if let Some(filter_id) = filter_window_id {
                if win_id != filter_id {
                    continue;
                }
            }

            let window_label = format!("W{}", idx + 1);

            let tabs = win.get("tabs").and_then(Value::as_array);
            let win_groups = win.get("groups").and_then(Value::as_array);

            if let Some(win_groups) = win_groups {
                for group in win_groups {
                    let group_id = group.get("groupId").and_then(Value::as_i64).unwrap_or(-1);
                    let tab_count = tabs
                        .map(|t| {
                            t.iter()
                                .filter(|tab| {
                                    tab.get("groupId").and_then(Value::as_i64) == Some(group_id)
                                })
                                .count()
                        })
                        .unwrap_or(0);

                    groups_out.push(serde_json::json!({
                        "windowId": win_id,
                        "windowLabel": window_label,
                        "groupId": group.get("groupId"),
                        "title": group.get("title"),
                        "color": group.get("color"),
                        "collapsed": group.get("collapsed"),
                        "tabCount": tab_count,
                    }));
                }
            }
        }

        OrchStep::Complete {
            response: serde_json::json!({ "groups": groups_out }),
            undo: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host_impl::orchestrate::Orchestration;

    fn sample_snapshot() -> Value {
        serde_json::json!({
            "generatedAt": 1_700_000_000_000_u64,
            "windows": [
                {
                    "windowId": 100,
                    "focused": true,
                    "state": "normal",
                    "tabs": [
                        {"tabId": 1, "windowId": 100, "index": 0, "url": "https://a.com", "title": "A", "active": true, "pinned": false, "groupId": 10, "groupTitle": "Work", "groupColor": "blue", "groupCollapsed": false, "lastFocusedAt": null},
                        {"tabId": 2, "windowId": 100, "index": 1, "url": "https://b.com", "title": "B", "active": false, "pinned": true, "groupId": 10, "groupTitle": "Work", "groupColor": "blue", "groupCollapsed": false, "lastFocusedAt": 1_700_000_000},
                        {"tabId": 3, "windowId": 100, "index": 2, "url": "https://c.com", "title": "C", "active": false, "pinned": false, "groupId": -1, "groupTitle": null, "groupColor": null, "groupCollapsed": null, "lastFocusedAt": null}
                    ],
                    "groups": [
                        {"groupId": 10, "title": "Work", "color": "blue", "collapsed": false}
                    ]
                }
            ]
        })
    }

    #[test]
    fn list_orchestration_shapes_snapshot() {
        let mut orch = ListOrchestration::new(&Value::Object(Map::new()));
        let step = orch.start();
        assert!(matches!(step, OrchStep::SendPrimitive { action, .. } if action == "p:snapshot"));

        let result = orch.step(sample_snapshot());
        let OrchStep::Complete { response, undo } = result else {
            panic!("expected Complete");
        };
        assert!(undo.is_none());

        let windows = response.get("windows").and_then(Value::as_array).unwrap();
        assert_eq!(windows.len(), 1);

        let win = &windows[0];
        assert_eq!(win.get("windowId").and_then(Value::as_i64), Some(100));
        assert_eq!(win.get("focused").and_then(Value::as_bool), Some(true));

        let tabs = win.get("tabs").and_then(Value::as_array).unwrap();
        assert_eq!(tabs.len(), 3);

        let tab = &tabs[0];
        assert_eq!(tab.get("tabId").and_then(Value::as_i64), Some(1));
        assert_eq!(
            tab.get("url").and_then(Value::as_str),
            Some("https://a.com")
        );
        assert_eq!(tab.get("groupTitle").and_then(Value::as_str), Some("Work"));

        // Verify dropped fields are absent
        assert!(tab.get("index").is_none());
        assert!(tab.get("pinned").is_none());
        assert!(tab.get("lastFocusedAt").is_none());
    }

    #[test]
    fn group_list_orchestration_counts_tabs() {
        let mut orch = GroupListOrchestration::new(&Value::Object(Map::new()));
        let step = orch.start();
        assert!(matches!(step, OrchStep::SendPrimitive { action, .. } if action == "p:snapshot"));

        let result = orch.step(sample_snapshot());
        let OrchStep::Complete { response, .. } = result else {
            panic!("expected Complete");
        };

        let groups = response.get("groups").and_then(Value::as_array).unwrap();
        assert_eq!(groups.len(), 1);

        let group = &groups[0];
        assert_eq!(group.get("groupId").and_then(Value::as_i64), Some(10));
        assert_eq!(group.get("title").and_then(Value::as_str), Some("Work"));
        assert_eq!(group.get("color").and_then(Value::as_str), Some("blue"));
        assert_eq!(group.get("tabCount").and_then(Value::as_i64), Some(2));
        assert_eq!(group.get("windowLabel").and_then(Value::as_str), Some("W1"));
    }

    #[test]
    fn group_list_filters_by_window_id() {
        let params = serde_json::json!({"windowId": 999});
        let mut orch = GroupListOrchestration::new(&params);
        let _ = orch.start();

        let result = orch.step(sample_snapshot());
        let OrchStep::Complete { response, .. } = result else {
            panic!("expected Complete");
        };

        let groups = response.get("groups").and_then(Value::as_array).unwrap();
        assert_eq!(groups.len(), 0);
    }
}
