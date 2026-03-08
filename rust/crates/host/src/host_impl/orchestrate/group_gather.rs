use serde_json::{Map, Value};

use super::OrchStep;

/// Orchestration for the `group-gather` command.
///
/// Merges duplicate groups (same title) within each window by moving tabs from
/// duplicate groups into the primary (earliest by tab index).
///
/// Multi-step: p:snapshot → for each duplicate group: p:tab-group → Complete.
#[derive(Debug)]
pub(crate) struct GroupGatherOrchestration {
    params: Value,
    phase: GatherPhase,
    state: Option<GatherState>,
}

#[derive(Debug)]
struct GatherState {
    /// Queue of pending merge operations: (primary_group_id, tab_ids_to_move)
    merge_queue: Vec<MergeOp>,
    current_idx: usize,
    merged: Vec<Value>,
    undo_tabs: Vec<Value>,
    has_incognito: bool,
}

#[derive(Debug)]
struct MergeOp {
    primary_group_id: i64,
    tab_ids: Vec<i64>,
}

#[derive(Debug)]
enum GatherPhase {
    GetSnapshot,
    MergeGroups,
}

impl GroupGatherOrchestration {
    pub(crate) fn new(params: &Value) -> Self {
        Self {
            params: params.clone(),
            phase: GatherPhase::GetSnapshot,
            state: None,
        }
    }
}

impl super::Orchestration for GroupGatherOrchestration {
    fn start(&mut self) -> OrchStep {
        OrchStep::SendPrimitive {
            action: "p:snapshot".to_string(),
            params: Value::Object(Map::new()),
        }
    }

    fn step(&mut self, response: Value) -> OrchStep {
        match self.phase {
            GatherPhase::GetSnapshot => self.handle_snapshot(response),
            GatherPhase::MergeGroups => self.handle_merged(response),
        }
    }
}

impl GroupGatherOrchestration {
    fn handle_snapshot(&mut self, snapshot: Value) -> OrchStep {
        let windows = snapshot
            .get("windows")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        let window_id_param = self
            .params
            .get("windowId")
            .and_then(|v| super::resolve::resolve_window_id(&snapshot, v));

        let group_title_filter = self
            .params
            .get("groupTitle")
            .and_then(Value::as_str)
            .map(|s| s.trim().to_string());

        let mut merge_queue = Vec::new();
        let mut merged_info = Vec::new();
        let mut undo_tabs = Vec::new();
        let mut has_incognito = false;

        for win in &windows {
            let win_id = win.get("windowId").and_then(Value::as_i64).unwrap_or(0);
            let window_incognito = win
                .get("incognito")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if matches!(window_id_param, Some(id) if id != win_id) {
                continue;
            }

            let groups = win.get("groups").and_then(Value::as_array);
            let tabs = win.get("tabs").and_then(Value::as_array);
            let Some(groups) = groups else { continue };
            let Some(tabs) = tabs else { continue };

            // Group by title
            let mut by_title: std::collections::HashMap<String, Vec<&Value>> =
                std::collections::HashMap::new();
            for group in groups {
                let title = group
                    .get("title")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if title.is_empty() {
                    continue;
                }
                if let Some(ref filter) = group_title_filter {
                    if &title != filter {
                        continue;
                    }
                }
                by_title.entry(title).or_default().push(group);
            }

            for (title, title_groups) in &by_title {
                if title_groups.len() < 2 {
                    continue;
                }

                // Sort groups by minimum tab index (earliest first)
                let mut groups_with_tabs: Vec<(&Value, Vec<&Value>, i64)> = title_groups
                    .iter()
                    .map(|g| {
                        let gid = g.get("groupId").and_then(Value::as_i64).unwrap_or(-1);
                        let group_tabs: Vec<&Value> = tabs
                            .iter()
                            .filter(|t| t.get("groupId").and_then(Value::as_i64) == Some(gid))
                            .collect();
                        let min_index = group_tabs
                            .iter()
                            .filter_map(|t| t.get("index").and_then(Value::as_i64))
                            .min()
                            .unwrap_or(i64::MAX);
                        (*g, group_tabs, min_index)
                    })
                    .collect();
                groups_with_tabs.sort_by_key(|(_, _, idx)| *idx);

                let primary = &groups_with_tabs[0];
                let primary_gid = primary
                    .0
                    .get("groupId")
                    .and_then(Value::as_i64)
                    .unwrap_or(-1);
                let duplicates = &groups_with_tabs[1..];
                let mut moved_tabs = 0;

                for dup in duplicates {
                    let dup_gid = dup.0.get("groupId").and_then(Value::as_i64).unwrap_or(-1);
                    let tab_ids: Vec<i64> = dup
                        .1
                        .iter()
                        .filter_map(|t| t.get("tabId").and_then(Value::as_i64))
                        .collect();

                    if tab_ids.is_empty() {
                        continue;
                    }

                    // Record undo entries
                    for tab in &dup.1 {
                        has_incognito |= tab
                            .get("incognito")
                            .and_then(Value::as_bool)
                            .unwrap_or(window_incognito);
                        undo_tabs.push(serde_json::json!({
                            "tabId": tab.get("tabId"),
                            "windowId": win_id,
                            "index": tab.get("index"),
                            "groupId": dup_gid,
                            "groupTitle": tab.get("groupTitle"),
                            "groupColor": tab.get("groupColor"),
                            "groupCollapsed": dup.0.get("collapsed"),
                        }));
                    }

                    moved_tabs += tab_ids.len();
                    merge_queue.push(MergeOp {
                        primary_group_id: primary_gid,
                        tab_ids,
                    });
                }

                merged_info.push(serde_json::json!({
                    "windowId": win_id,
                    "groupTitle": title,
                    "primaryGroupId": primary_gid,
                    "mergedGroupCount": duplicates.len(),
                    "movedTabs": moved_tabs,
                }));
            }
        }

        if merge_queue.is_empty() {
            return OrchStep::Complete {
                response: serde_json::json!({
                    "merged": merged_info,
                    "summary": { "mergedGroups": 0, "movedTabs": 0 },
                }),
                undo: Some(serde_json::json!({
                    "action": "group-gather",
                    "tabs": [],
                })),
            };
        }

        self.state = Some(GatherState {
            merge_queue,
            current_idx: 0,
            merged: merged_info,
            undo_tabs,
            has_incognito,
        });
        self.phase = GatherPhase::MergeGroups;
        self.send_next_merge()
    }

    fn handle_merged(&mut self, _response: Value) -> OrchStep {
        let state = self.state.as_mut().unwrap();
        state.current_idx += 1;

        if state.current_idx < state.merge_queue.len() {
            self.send_next_merge()
        } else {
            self.complete()
        }
    }

    fn send_next_merge(&self) -> OrchStep {
        let state = self.state.as_ref().unwrap();
        let op = &state.merge_queue[state.current_idx];
        OrchStep::SendPrimitive {
            action: "p:tab-group".to_string(),
            params: serde_json::json!({
                "groupId": op.primary_group_id,
                "tabIds": op.tab_ids,
            }),
        }
    }

    fn complete(&self) -> OrchStep {
        let state = self.state.as_ref().unwrap();

        let total_merged: usize = state
            .merged
            .iter()
            .filter_map(|m| m.get("mergedGroupCount").and_then(Value::as_u64))
            .map(|v| v as usize)
            .sum();
        let total_moved: usize = state
            .merged
            .iter()
            .filter_map(|m| m.get("movedTabs").and_then(Value::as_u64))
            .map(|v| v as usize)
            .sum();

        OrchStep::Complete {
            response: serde_json::json!({
                "merged": state.merged,
                "summary": {
                    "mergedGroups": total_merged,
                    "movedTabs": total_moved,
                },
            }),
            undo: Some(serde_json::json!({
                "action": "group-gather",
                "incognito": state.has_incognito,
                "tabs": state.undo_tabs,
            })),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host_impl::orchestrate::Orchestration;

    fn snapshot_with_duplicates() -> Value {
        serde_json::json!({
            "windows": [{
                "windowId": 100,
                "focused": true,
                "tabs": [
                    {"tabId": 1, "windowId": 100, "index": 0, "groupId": 10, "groupTitle": "Dev", "groupColor": "blue", "groupCollapsed": false},
                    {"tabId": 2, "windowId": 100, "index": 1, "groupId": 10, "groupTitle": "Dev", "groupColor": "blue", "groupCollapsed": false},
                    {"tabId": 3, "windowId": 100, "index": 2, "groupId": -1},
                    {"tabId": 4, "windowId": 100, "index": 3, "groupId": 20, "groupTitle": "Dev", "groupColor": "red", "groupCollapsed": false},
                    {"tabId": 5, "windowId": 100, "index": 4, "groupId": 20, "groupTitle": "Dev", "groupColor": "red", "groupCollapsed": false}
                ],
                "groups": [
                    {"groupId": 10, "title": "Dev", "color": "blue", "collapsed": false},
                    {"groupId": 20, "title": "Dev", "color": "red", "collapsed": false}
                ]
            }]
        })
    }

    #[test]
    fn gather_merges_duplicate_groups() {
        let params = serde_json::json!({});
        let mut orch = GroupGatherOrchestration::new(&params);

        let step = orch.start();
        assert!(matches!(&step, OrchStep::SendPrimitive { action, .. } if action == "p:snapshot"));

        let step = orch.step(snapshot_with_duplicates());
        let OrchStep::SendPrimitive { action, params } = &step else {
            panic!("expected tab-group, got {step:?}");
        };
        assert_eq!(action, "p:tab-group");
        assert_eq!(params["groupId"], 10); // primary (earliest)
        let tab_ids = params["tabIds"].as_array().unwrap();
        assert_eq!(tab_ids.len(), 2); // tabs 4 and 5

        let step = orch.step(serde_json::json!({"groupId": 10}));
        let OrchStep::Complete { response, undo } = step else {
            panic!("expected Complete");
        };
        assert_eq!(response["summary"]["mergedGroups"], 1);
        assert_eq!(response["summary"]["movedTabs"], 2);

        let undo = undo.unwrap();
        assert_eq!(undo["action"], "group-gather");
        assert_eq!(undo["tabs"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn gather_no_duplicates_completes_immediately() {
        let snapshot = serde_json::json!({
            "windows": [{
                "windowId": 100,
                "focused": true,
                "tabs": [
                    {"tabId": 1, "windowId": 100, "index": 0, "groupId": 10}
                ],
                "groups": [
                    {"groupId": 10, "title": "Dev", "color": "blue", "collapsed": false}
                ]
            }]
        });

        let params = serde_json::json!({});
        let mut orch = GroupGatherOrchestration::new(&params);
        let _ = orch.start();
        let step = orch.step(snapshot);
        let OrchStep::Complete { response, .. } = step else {
            panic!("expected Complete");
        };
        assert_eq!(response["summary"]["mergedGroups"], 0);
    }

    #[test]
    fn gather_filters_by_group_title() {
        let params = serde_json::json!({"groupTitle": "NonExistent"});
        let mut orch = GroupGatherOrchestration::new(&params);
        let _ = orch.start();
        let step = orch.step(snapshot_with_duplicates());
        let OrchStep::Complete { response, .. } = step else {
            panic!("expected Complete");
        };
        assert_eq!(response["summary"]["mergedGroups"], 0);
    }
}
