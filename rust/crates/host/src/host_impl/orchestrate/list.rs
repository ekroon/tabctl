use serde_json::{Map, Value};

use super::resolve::resolve_window_id;
use super::scope::{select_tabs_by_scope, ScopedTab};
use super::OrchStep;

/// Default page size when no explicit --limit is given.
const DEFAULT_LIMIT: usize = 20;

/// Orchestration for the `list` command.
///
/// Single step: request p:snapshot, filter by scope, apply pagination
/// (default limit 20), then shape the response with a `pagination` object.
#[derive(Debug)]
pub(crate) struct ListOrchestration {
    params: Value,
}

impl ListOrchestration {
    pub(crate) fn new(params: &Value) -> Self {
        Self {
            params: params.clone(),
        }
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

        self.paginated_response(&response, windows)
    }
}

impl ListOrchestration {
    fn paginated_response(&self, snapshot: &Value, windows: &[Value]) -> OrchStep {
        let scope_result = select_tabs_by_scope(snapshot, &self.params);

        if let Some(err) = scope_result.error {
            return OrchStep::Error {
                message: err,
                hint: None,
            };
        }

        let mut tabs = scope_result.tabs;
        let total_tabs = tabs.len();

        // Sort tabs before pagination
        let sort_field = self
            .params
            .get("sort")
            .and_then(Value::as_str)
            .unwrap_or("index");
        match sort_field {
            "last-accessed" => {
                tabs.sort_by(|a, b| {
                    let ta = a.last_accessed_at.unwrap_or(0);
                    let tb = b.last_accessed_at.unwrap_or(0);
                    tb.cmp(&ta) // descending: most recent first
                });
            }
            "title" => {
                tabs.sort_by(|a, b| {
                    let ta = a.title.as_deref().unwrap_or("");
                    let tb = b.title.as_deref().unwrap_or("");
                    ta.to_lowercase().cmp(&tb.to_lowercase())
                });
            }
            "url" => {
                tabs.sort_by(|a, b| {
                    let ua = a.url.as_deref().unwrap_or("");
                    let ub = b.url.as_deref().unwrap_or("");
                    ua.to_lowercase().cmp(&ub.to_lowercase())
                });
            }
            _ => {} // "index" — default order from snapshot
        }

        // Apply offset
        let offset = self
            .params
            .get("offset")
            .and_then(Value::as_u64)
            .unwrap_or(0) as usize;
        if offset > 0 && offset < tabs.len() {
            tabs = tabs.split_off(offset);
        } else if offset >= tabs.len() && !tabs.is_empty() {
            tabs.clear();
        }

        // Apply limit (default 20)
        let limit = self
            .params
            .get("limit")
            .and_then(Value::as_u64)
            .map(|l| l as usize)
            .unwrap_or(DEFAULT_LIMIT);
        tabs.truncate(limit);

        // Reconstruct window structure from filtered tabs
        let shaped = reconstruct_windows(&tabs, windows);

        let has_more = offset + tabs.len() < total_tabs;
        let response = serde_json::json!({
            "windows": shaped,
            "pagination": {
                "totalTabs": total_tabs,
                "offset": offset,
                "count": tabs.len(),
                "hasMore": has_more,
            },
        });

        OrchStep::Complete {
            response,
            undo: None,
        }
    }
}

/// Reconstruct windows from scoped tabs, preserving tab order.
///
/// Window emission order follows first-appearance of each window in the
/// (already-sorted) tab list, so that `--sort last-accessed` keeps the
/// most-recently-accessed window first.
fn reconstruct_windows(tabs: &[ScopedTab], windows: &[Value]) -> Vec<Value> {
    // Collect unique window IDs in first-appearance order
    let mut seen = std::collections::HashSet::new();
    let mut win_order: Vec<i64> = Vec::new();
    for tab in tabs {
        if seen.insert(tab.window_id) {
            win_order.push(tab.window_id);
        }
    }

    let mut result = Vec::new();
    for win_id in &win_order {
        let win_tabs: Vec<&ScopedTab> = tabs.iter().filter(|t| t.window_id == *win_id).collect();

        let win_meta = windows
            .iter()
            .find(|w| w.get("windowId").and_then(Value::as_i64) == Some(*win_id));

        let focused = win_meta
            .and_then(|w| w.get("focused").and_then(Value::as_bool))
            .unwrap_or(false);

        let tab_values: Vec<Value> = win_tabs.iter().map(|t| shape_scoped_tab(t)).collect();

        // Include only groups that have matching tabs
        let group_ids: std::collections::HashSet<i64> =
            win_tabs.iter().map(|t| t.group_id).collect();
        let groups: Vec<Value> = win_meta
            .and_then(|w| w.get("groups").and_then(Value::as_array))
            .map(|groups| {
                groups
                    .iter()
                    .filter(|g| {
                        g.get("groupId")
                            .and_then(Value::as_i64)
                            .is_some_and(|id| group_ids.contains(&id))
                    })
                    .map(shape_group)
                    .collect()
            })
            .unwrap_or_default();

        result.push(serde_json::json!({
            "windowId": win_id,
            "focused": focused,
            "tabs": tab_values,
            "groups": groups,
        }));
    }
    result
}

fn shape_scoped_tab(t: &ScopedTab) -> Value {
    let mut obj = serde_json::json!({
        "tabId": t.tab_id,
        "windowId": t.window_id,
        "url": t.url,
        "title": t.title,
        "active": t.active,
        "pinned": t.pinned,
        "index": t.index,
        "groupId": t.group_id,
        "groupTitle": t.group_title,
    });
    if let Some(ts) = t.last_accessed_at {
        obj["lastAccessedAt"] = Value::Number(ts.into());
    }
    obj
}

fn shape_group(g: &Value) -> Value {
    serde_json::json!({
        "groupId": g.get("groupId"),
        "title": g.get("title"),
        "color": g.get("color"),
        "collapsed": g.get("collapsed"),
    })
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
        let filter_window_id = self
            .window_id_param
            .as_ref()
            .and_then(|v| resolve_window_id(&response, v));

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

/// Orchestration for the `snapshot` action.
///
/// Single step: sends p:snapshot and returns the raw response unchanged.
/// Used by the CLI for client-side filtering/pagination.
#[derive(Debug)]
pub(crate) struct SnapshotOrchestration;

impl super::Orchestration for SnapshotOrchestration {
    fn start(&mut self) -> OrchStep {
        OrchStep::SendPrimitive {
            action: "p:snapshot".to_string(),
            params: Value::Object(Map::new()),
        }
    }

    fn step(&mut self, response: Value) -> OrchStep {
        OrchStep::Complete {
            response,
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
                        {"tabId": 1, "windowId": 100, "index": 0, "url": "https://a.com", "title": "A", "active": true, "pinned": false, "groupId": 10, "groupTitle": "Work", "groupColor": "blue", "groupCollapsed": false, "lastAccessedAt": null},
                        {"tabId": 2, "windowId": 100, "index": 1, "url": "https://b.com", "title": "B", "active": false, "pinned": true, "groupId": 10, "groupTitle": "Work", "groupColor": "blue", "groupCollapsed": false, "lastAccessedAt": 1_700_000_000},
                        {"tabId": 3, "windowId": 100, "index": 2, "url": "https://c.com", "title": "C", "active": false, "pinned": false, "groupId": -1, "groupTitle": null, "groupColor": null, "groupCollapsed": null, "lastAccessedAt": null}
                    ],
                    "groups": [
                        {"groupId": 10, "title": "Work", "color": "blue", "collapsed": false}
                    ]
                },
                {
                    "windowId": 200,
                    "focused": false,
                    "state": "normal",
                    "tabs": [
                        {"tabId": 4, "windowId": 200, "index": 0, "url": "https://d.com", "title": "D", "active": true, "pinned": false, "groupId": -1, "groupTitle": null}
                    ],
                    "groups": []
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
        // Default scope = focused window only
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

        // Verify enriched fields are present, dropped fields are absent
        assert!(tab.get("index").is_some());
        assert!(tab.get("pinned").is_some());
        // Tab 1 has null lastAccessedAt, so not included
        assert!(tab.get("lastAccessedAt").is_none());
        // Tab 2 has a timestamp, so it should be included
        let tab2 = &tabs[1];
        assert_eq!(
            tab2.get("lastAccessedAt").and_then(Value::as_i64),
            Some(1_700_000_000)
        );

        // Pagination always present
        let pg = response.get("pagination").unwrap();
        assert_eq!(pg.get("totalTabs").and_then(Value::as_u64), Some(3));
        assert_eq!(pg.get("offset").and_then(Value::as_u64), Some(0));
        assert_eq!(pg.get("count").and_then(Value::as_u64), Some(3));
        assert_eq!(pg.get("hasMore").and_then(Value::as_bool), Some(false));
    }

    #[test]
    fn list_filters_by_window_id() {
        let params = serde_json::json!({"windowId": 200});
        let mut orch = ListOrchestration::new(&params);
        let _ = orch.start();

        let result = orch.step(sample_snapshot());
        let OrchStep::Complete { response, .. } = result else {
            panic!("expected Complete");
        };

        let windows = response.get("windows").and_then(Value::as_array).unwrap();
        assert_eq!(windows.len(), 1);
        assert_eq!(
            windows[0].get("windowId").and_then(Value::as_i64),
            Some(200)
        );

        let tabs = windows[0].get("tabs").and_then(Value::as_array).unwrap();
        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs[0].get("tabId").and_then(Value::as_i64), Some(4));
    }

    #[test]
    fn list_filters_by_tab_ids() {
        let params = serde_json::json!({"tabIds": [2, 4]});
        let mut orch = ListOrchestration::new(&params);
        let _ = orch.start();

        let result = orch.step(sample_snapshot());
        let OrchStep::Complete { response, .. } = result else {
            panic!("expected Complete");
        };

        let windows = response.get("windows").and_then(Value::as_array).unwrap();
        // Two tabs from different windows
        assert_eq!(windows.len(), 2);

        let all_tab_ids: Vec<i64> = windows
            .iter()
            .flat_map(|w| {
                w.get("tabs")
                    .and_then(Value::as_array)
                    .unwrap()
                    .iter()
                    .map(|t| t.get("tabId").and_then(Value::as_i64).unwrap())
            })
            .collect();
        assert_eq!(all_tab_ids, vec![2, 4]);
    }

    #[test]
    fn list_filters_by_group_title() {
        let params = serde_json::json!({"groupTitle": "Work"});
        let mut orch = ListOrchestration::new(&params);
        let _ = orch.start();

        let result = orch.step(sample_snapshot());
        let OrchStep::Complete { response, .. } = result else {
            panic!("expected Complete");
        };

        let windows = response.get("windows").and_then(Value::as_array).unwrap();
        assert_eq!(windows.len(), 1);

        let tabs = windows[0].get("tabs").and_then(Value::as_array).unwrap();
        assert_eq!(tabs.len(), 2);
        assert!(tabs
            .iter()
            .all(|t| t.get("groupTitle").and_then(Value::as_str) == Some("Work")));

        // Groups should only include the Work group
        let groups = windows[0].get("groups").and_then(Value::as_array).unwrap();
        assert_eq!(groups.len(), 1);
    }

    #[test]
    fn list_all_returns_everything() {
        let params = serde_json::json!({"all": true});
        let mut orch = ListOrchestration::new(&params);
        let _ = orch.start();

        let result = orch.step(sample_snapshot());
        let OrchStep::Complete { response, .. } = result else {
            panic!("expected Complete");
        };

        let windows = response.get("windows").and_then(Value::as_array).unwrap();
        let total_tabs: usize = windows
            .iter()
            .map(|w| {
                w.get("tabs")
                    .and_then(Value::as_array)
                    .map(|t| t.len())
                    .unwrap_or(0)
            })
            .sum();
        assert_eq!(total_tabs, 4);

        // Pagination always present
        let pg = response.get("pagination").unwrap();
        assert_eq!(pg.get("totalTabs").and_then(Value::as_u64), Some(4));
        assert_eq!(pg.get("hasMore").and_then(Value::as_bool), Some(false));
    }

    #[test]
    fn list_limit_truncates() {
        let params = serde_json::json!({"all": true, "limit": 2});
        let mut orch = ListOrchestration::new(&params);
        let _ = orch.start();

        let result = orch.step(sample_snapshot());
        let OrchStep::Complete { response, .. } = result else {
            panic!("expected Complete");
        };

        let total_tabs: usize = response
            .get("windows")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .map(|w| {
                w.get("tabs")
                    .and_then(Value::as_array)
                    .map(|t| t.len())
                    .unwrap_or(0)
            })
            .sum();
        assert_eq!(total_tabs, 2);

        let pg = response.get("pagination").unwrap();
        assert_eq!(pg.get("totalTabs").and_then(Value::as_u64), Some(4));
        assert_eq!(pg.get("offset").and_then(Value::as_u64), Some(0));
        assert_eq!(pg.get("count").and_then(Value::as_u64), Some(2));
        assert_eq!(pg.get("hasMore").and_then(Value::as_bool), Some(true));
    }

    #[test]
    fn list_offset_skips() {
        let params = serde_json::json!({"all": true, "offset": 2});
        let mut orch = ListOrchestration::new(&params);
        let _ = orch.start();

        let result = orch.step(sample_snapshot());
        let OrchStep::Complete { response, .. } = result else {
            panic!("expected Complete");
        };

        let all_tabs: Vec<i64> = response
            .get("windows")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .flat_map(|w| {
                w.get("tabs")
                    .and_then(Value::as_array)
                    .unwrap()
                    .iter()
                    .map(|t| t.get("tabId").and_then(Value::as_i64).unwrap())
            })
            .collect();
        // Tabs 1,2 skipped; tabs 3,4 remain
        assert_eq!(all_tabs, vec![3, 4]);

        let pg = response.get("pagination").unwrap();
        assert_eq!(pg.get("totalTabs").and_then(Value::as_u64), Some(4));
        assert_eq!(pg.get("offset").and_then(Value::as_u64), Some(2));
        assert_eq!(pg.get("count").and_then(Value::as_u64), Some(2));
        assert_eq!(pg.get("hasMore").and_then(Value::as_bool), Some(false));
    }

    #[test]
    fn list_limit_and_offset_combined() {
        let params = serde_json::json!({"all": true, "offset": 1, "limit": 2});
        let mut orch = ListOrchestration::new(&params);
        let _ = orch.start();

        let result = orch.step(sample_snapshot());
        let OrchStep::Complete { response, .. } = result else {
            panic!("expected Complete");
        };

        let all_tabs: Vec<i64> = response
            .get("windows")
            .and_then(Value::as_array)
            .unwrap()
            .iter()
            .flat_map(|w| {
                w.get("tabs")
                    .and_then(Value::as_array)
                    .unwrap()
                    .iter()
                    .map(|t| t.get("tabId").and_then(Value::as_i64).unwrap())
            })
            .collect();
        // Skip 1 (tab 1), take 2 (tabs 2, 3)
        assert_eq!(all_tabs, vec![2, 3]);

        let pg = response.get("pagination").unwrap();
        assert_eq!(pg.get("totalTabs").and_then(Value::as_u64), Some(4));
        assert_eq!(pg.get("offset").and_then(Value::as_u64), Some(1));
        assert_eq!(pg.get("count").and_then(Value::as_u64), Some(2));
        assert_eq!(pg.get("hasMore").and_then(Value::as_bool), Some(true));
    }

    #[test]
    fn list_scoped_includes_matching_groups_only() {
        let params = serde_json::json!({"tabIds": [3]});
        let mut orch = ListOrchestration::new(&params);
        let _ = orch.start();

        let result = orch.step(sample_snapshot());
        let OrchStep::Complete { response, .. } = result else {
            panic!("expected Complete");
        };

        let windows = response.get("windows").and_then(Value::as_array).unwrap();
        assert_eq!(windows.len(), 1);

        // Tab 3 has groupId -1, so Work group should not be included
        let groups = windows[0].get("groups").and_then(Value::as_array).unwrap();
        assert_eq!(groups.len(), 0);
    }

    #[test]
    fn list_default_limit_caps_at_20() {
        // Build a snapshot with 25 tabs in one window
        let tabs: Vec<Value> = (1..=25)
            .map(|i| {
                serde_json::json!({
                    "tabId": i, "windowId": 100, "index": i - 1,
                    "url": format!("https://t{i}.com"), "title": format!("T{i}"),
                    "active": i == 1, "pinned": false, "groupId": -1,
                })
            })
            .collect();
        let snapshot = serde_json::json!({
            "windows": [{"windowId": 100, "focused": true, "tabs": tabs, "groups": []}]
        });

        let params = serde_json::json!({"all": true});
        let mut orch = ListOrchestration::new(&params);
        let _ = orch.start();

        let result = orch.step(snapshot);
        let OrchStep::Complete { response, .. } = result else {
            panic!("expected Complete");
        };

        let pg = response.get("pagination").unwrap();
        assert_eq!(pg.get("totalTabs").and_then(Value::as_u64), Some(25));
        assert_eq!(pg.get("count").and_then(Value::as_u64), Some(20));
        assert_eq!(pg.get("hasMore").and_then(Value::as_bool), Some(true));
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

    #[test]
    fn snapshot_orchestration_returns_raw_response() {
        let mut orch = SnapshotOrchestration;
        let step = orch.start();
        assert!(matches!(&step, OrchStep::SendPrimitive { action, .. } if action == "p:snapshot"));

        let snap = sample_snapshot();
        let result = orch.step(snap.clone());
        let OrchStep::Complete { response, undo } = result else {
            panic!("expected Complete");
        };
        assert!(undo.is_none());
        // Raw response returned unchanged
        assert_eq!(response, snap);
    }

    fn snapshot_with_timestamps() -> Value {
        serde_json::json!({
            "generatedAt": 1_700_000_000_000_u64,
            "windows": [
                {
                    "windowId": 100,
                    "focused": true,
                    "state": "normal",
                    "tabs": [
                        {"tabId": 1, "windowId": 100, "index": 0, "url": "https://z.com", "title": "Zebra", "active": false, "pinned": false, "groupId": -1, "lastAccessedAt": 1000},
                        {"tabId": 2, "windowId": 100, "index": 1, "url": "https://a.com", "title": "Apple", "active": false, "pinned": false, "groupId": -1, "lastAccessedAt": 3000},
                        {"tabId": 3, "windowId": 100, "index": 2, "url": "https://m.com", "title": "Mango", "active": true, "pinned": false, "groupId": -1, "lastAccessedAt": 2000},
                        {"tabId": 4, "windowId": 100, "index": 3, "url": "https://b.com", "title": "Banana", "active": false, "pinned": false, "groupId": -1, "lastAccessedAt": null}
                    ],
                    "groups": []
                }
            ]
        })
    }

    fn tab_ids_in_order(response: &Value) -> Vec<i64> {
        response["windows"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|w| w["tabs"].as_array().unwrap())
            .map(|t| t["tabId"].as_i64().unwrap())
            .collect()
    }

    #[test]
    fn sort_last_accessed_orders_by_most_recent_first() {
        let params = serde_json::json!({"all": true, "sort": "last-accessed"});
        let mut orch = ListOrchestration::new(&params);
        orch.start();
        let result = orch.step(snapshot_with_timestamps());
        let OrchStep::Complete { response, .. } = result else {
            panic!("expected Complete");
        };
        // 3000, 2000, 1000, null(0)
        assert_eq!(tab_ids_in_order(&response), vec![2, 3, 1, 4]);
    }

    #[test]
    fn sort_title_orders_alphabetically() {
        let params = serde_json::json!({"all": true, "sort": "title"});
        let mut orch = ListOrchestration::new(&params);
        orch.start();
        let result = orch.step(snapshot_with_timestamps());
        let OrchStep::Complete { response, .. } = result else {
            panic!("expected Complete");
        };
        // Apple, Banana, Mango, Zebra
        assert_eq!(tab_ids_in_order(&response), vec![2, 4, 3, 1]);
    }

    #[test]
    fn sort_url_orders_alphabetically() {
        let params = serde_json::json!({"all": true, "sort": "url"});
        let mut orch = ListOrchestration::new(&params);
        orch.start();
        let result = orch.step(snapshot_with_timestamps());
        let OrchStep::Complete { response, .. } = result else {
            panic!("expected Complete");
        };
        // a.com, b.com, m.com, z.com
        assert_eq!(tab_ids_in_order(&response), vec![2, 4, 3, 1]);
    }

    #[test]
    fn sort_index_preserves_original_order() {
        let params = serde_json::json!({"all": true, "sort": "index"});
        let mut orch = ListOrchestration::new(&params);
        orch.start();
        let result = orch.step(snapshot_with_timestamps());
        let OrchStep::Complete { response, .. } = result else {
            panic!("expected Complete");
        };
        // Original snapshot order
        assert_eq!(tab_ids_in_order(&response), vec![1, 2, 3, 4]);
    }

    #[test]
    fn default_sort_is_index_order() {
        let params = serde_json::json!({"all": true});
        let mut orch = ListOrchestration::new(&params);
        orch.start();
        let result = orch.step(snapshot_with_timestamps());
        let OrchStep::Complete { response, .. } = result else {
            panic!("expected Complete");
        };
        // No sort param = index order
        assert_eq!(tab_ids_in_order(&response), vec![1, 2, 3, 4]);
    }

    #[test]
    fn sort_last_accessed_preserves_window_order_across_windows() {
        // Tab 5 (window 200) has the most recent timestamp, so window 200
        // must appear before window 100 in the sorted output.
        let snap = serde_json::json!({
            "generatedAt": 1_700_000_000_000_u64,
            "windows": [
                {
                    "windowId": 100,
                    "focused": true,
                    "state": "normal",
                    "tabs": [
                        {"tabId": 1, "windowId": 100, "index": 0, "url": "https://a.com", "title": "A", "active": false, "pinned": false, "groupId": -1, "lastAccessedAt": 1000},
                        {"tabId": 2, "windowId": 100, "index": 1, "url": "https://b.com", "title": "B", "active": false, "pinned": false, "groupId": -1, "lastAccessedAt": 2000}
                    ],
                    "groups": []
                },
                {
                    "windowId": 200,
                    "focused": false,
                    "state": "normal",
                    "tabs": [
                        {"tabId": 5, "windowId": 200, "index": 0, "url": "https://e.com", "title": "E", "active": true, "pinned": false, "groupId": -1, "lastAccessedAt": 9000}
                    ],
                    "groups": []
                }
            ]
        });

        let params = serde_json::json!({"all": true, "sort": "last-accessed"});
        let mut orch = ListOrchestration::new(&params);
        orch.start();
        let result = orch.step(snap);
        let OrchStep::Complete { response, .. } = result else {
            panic!("expected Complete");
        };

        // Global sort: 9000, 2000, 1000 → tab 5 first, so window 200 first
        let windows = response["windows"].as_array().unwrap();
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0]["windowId"], 200);
        assert_eq!(windows[1]["windowId"], 100);
        // Tab order within windows is preserved by sort
        assert_eq!(tab_ids_in_order(&response), vec![5, 2, 1]);
    }
}
