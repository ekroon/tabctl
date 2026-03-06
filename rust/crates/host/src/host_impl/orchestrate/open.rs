use serde_json::{Map, Value};
use tabctl_shared::normalize_url;

use super::resolve::resolve_window_id;
use super::OrchStep;

/// Orchestration for the `open` command.
///
/// Two main paths:
/// 1. `--new-window`: p:window-create → p:tab-create×N → p:tab-remove(seed)
///    → p:tab-group + p:group-update
/// 2. Existing window: p:snapshot → resolve window + dedup URLs →
///    p:tab-create×N → p:tab-group + p:group-update → p:tab-query → verify
#[derive(Debug)]
pub(crate) struct OpenOrchestration {
    params: Value,
    phase: OpenPhase,
    state: OpenState,
}

#[derive(Debug, Default)]
struct OpenState {
    window_id: Option<i64>,
    seed_tab_id: Option<i64>,
    existing_group_id: Option<i64>,
    group_title: Option<String>,
    group_color: Option<String>,
    force_new_group: bool,
    urls: Vec<String>,
    url_idx: usize,
    created: Vec<Value>,
    skipped: Vec<Value>,
    new_group_id: Option<i64>,
    need_group_update: bool,
    insert_index: Option<i64>,
}

#[derive(Debug)]
enum OpenPhase {
    Init,
    Snapshot,
    CreateTabs,
    RemoveSeedTab,
    GroupTabs,
    UpdateGroup,
    VerifyQuery,
    VerifyGroup,
}

impl OpenOrchestration {
    pub(crate) fn new(params: &Value) -> Self {
        Self {
            params: params.clone(),
            phase: OpenPhase::Init,
            state: OpenState::default(),
        }
    }
}

impl super::Orchestration for OpenOrchestration {
    fn start(&mut self) -> OrchStep {
        // Parse params
        let urls: Vec<String> = self
            .params
            .get("urls")
            .and_then(Value::as_array)
            .map(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect()
            })
            .unwrap_or_default();

        let group_title = self
            .params
            .get("groupTitle")
            .and_then(Value::as_str)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let group_color = self
            .params
            .get("color")
            .and_then(Value::as_str)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let new_window = self
            .params
            .get("newWindow")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let force_new_group = self
            .params
            .get("newGroup")
            .and_then(Value::as_bool)
            .unwrap_or(false);

        if urls.is_empty() && !new_window {
            return OrchStep::Error {
                message: "No URLs provided".to_string(),
                hint: None,
            };
        }

        self.state.urls = urls;
        self.state.group_title = group_title;
        self.state.group_color = group_color;
        self.state.force_new_group = force_new_group;

        if new_window {
            self.phase = OpenPhase::Init;
            OrchStep::SendPrimitive {
                action: "p:window-create".to_string(),
                params: serde_json::json!({"focused": false}),
            }
        } else {
            self.phase = OpenPhase::Snapshot;
            OrchStep::SendPrimitive {
                action: "p:snapshot".to_string(),
                params: Value::Object(Map::new()),
            }
        }
    }

    fn step(&mut self, response: Value) -> OrchStep {
        match self.phase {
            OpenPhase::Init => self.handle_window_created(response),
            OpenPhase::Snapshot => self.handle_snapshot(response),
            OpenPhase::CreateTabs => self.handle_tab_created(response),
            OpenPhase::RemoveSeedTab => self.handle_seed_removed(),
            OpenPhase::GroupTabs => self.handle_grouped(response),
            OpenPhase::UpdateGroup => self.handle_group_updated(),
            OpenPhase::VerifyQuery => self.handle_verify_query(response),
            OpenPhase::VerifyGroup => self.complete(),
        }
    }
}

impl OpenOrchestration {
    fn handle_window_created(&mut self, response: Value) -> OrchStep {
        let window_id = response
            .get("id")
            .or_else(|| response.get("windowId"))
            .and_then(Value::as_i64);

        let Some(window_id) = window_id else {
            return OrchStep::Error {
                message: "Failed to create window".to_string(),
                hint: None,
            };
        };

        self.state.window_id = Some(window_id);

        // Find seed tab from the new window
        self.state.seed_tab_id = response
            .get("tabs")
            .and_then(Value::as_array)
            .and_then(|tabs| tabs.first())
            .and_then(|tab| tab.get("id").and_then(Value::as_i64));

        self.next_tab_create()
    }

    fn handle_snapshot(&mut self, snapshot: Value) -> OrchStep {
        let group_title = self.state.group_title.as_deref();
        let force_new_group = self.state.force_new_group;

        // Resolve window
        let mut effective_params = self.params.clone();

        // Auto-resolve window by group name
        if let Some(gt) = group_title {
            if !force_new_group
                && self.params.get("windowId").is_none()
                && self.params.get("windowGroupTitle").is_none()
            {
                let group_windows: Vec<i64> = snapshot
                    .get("windows")
                    .and_then(Value::as_array)
                    .map(|wins| {
                        wins.iter()
                            .filter(|w| {
                                w.get("groups")
                                    .and_then(Value::as_array)
                                    .map(|gs| {
                                        gs.iter().any(|g| {
                                            g.get("title").and_then(Value::as_str) == Some(gt)
                                        })
                                    })
                                    .unwrap_or(false)
                            })
                            .filter_map(|w| w.get("windowId").and_then(Value::as_i64))
                            .collect()
                    })
                    .unwrap_or_default();

                if group_windows.len() == 1 {
                    if let Some(obj) = effective_params.as_object_mut() {
                        obj.insert(
                            "windowId".to_string(),
                            Value::Number(group_windows[0].into()),
                        );
                    }
                }
            }
        }

        // Resolve window ID
        let window_id = if let Some(raw) = effective_params.get("windowId") {
            resolve_window_id(&snapshot, raw)
        } else {
            // Default to focused window
            snapshot
                .get("windows")
                .and_then(Value::as_array)
                .and_then(|wins| {
                    wins.iter().find_map(|w| {
                        if w.get("focused").and_then(Value::as_bool) == Some(true) {
                            w.get("windowId").and_then(Value::as_i64)
                        } else {
                            None
                        }
                    })
                })
        };

        let Some(window_id) = window_id else {
            return OrchStep::Error {
                message: "Target window not found".to_string(),
                hint: None,
            };
        };

        self.state.window_id = Some(window_id);

        // Find existing group for reuse
        if let Some(gt) = group_title {
            if !force_new_group {
                let window_snap =
                    snapshot
                        .get("windows")
                        .and_then(Value::as_array)
                        .and_then(|wins| {
                            wins.iter().find(|w| {
                                w.get("windowId").and_then(Value::as_i64) == Some(window_id)
                            })
                        });

                if let Some(win) = window_snap {
                    let matching: Vec<i64> = win
                        .get("groups")
                        .and_then(Value::as_array)
                        .map(|gs| {
                            gs.iter()
                                .filter(|g| g.get("title").and_then(Value::as_str) == Some(gt))
                                .filter_map(|g| g.get("groupId").and_then(Value::as_i64))
                                .collect()
                        })
                        .unwrap_or_default();

                    if matching.len() > 1 {
                        return OrchStep::Error {
                            message: format!(
                                "Ambiguous group title \"{gt}\": found {} groups with the same name.",
                                matching.len()
                            ),
                            hint: Some(
                                "Use --new-group to force a new group, group-gather to merge, \
                                 or --group-id to target by ID."
                                    .to_string(),
                            ),
                        };
                    }

                    if matching.len() == 1 {
                        let gid = matching[0];
                        self.state.existing_group_id = Some(gid);

                        // Filter duplicate URLs
                        let allow_duplicates = self
                            .params
                            .get("allowDuplicates")
                            .and_then(Value::as_bool)
                            .unwrap_or(false);

                        if !allow_duplicates {
                            let existing_urls: std::collections::HashSet<String> = win
                                .get("tabs")
                                .and_then(Value::as_array)
                                .map(|tabs| {
                                    tabs.iter()
                                        .filter(|t| {
                                            t.get("groupId").and_then(Value::as_i64) == Some(gid)
                                        })
                                        .filter_map(|t| t.get("url").and_then(Value::as_str))
                                        .filter(|u| !u.trim().is_empty())
                                        .map(normalize_url)
                                        .collect()
                                })
                                .unwrap_or_default();

                            let mut filtered_urls = Vec::new();
                            for url in &self.state.urls {
                                let trimmed = url.trim();
                                if !trimmed.is_empty() {
                                    let norm = normalize_url(trimmed);
                                    if existing_urls.contains(&norm) {
                                        self.state.skipped.push(
                                            serde_json::json!({"url": url, "reason": "duplicate"}),
                                        );
                                        continue;
                                    }
                                }
                                filtered_urls.push(url.clone());
                            }
                            self.state.urls = filtered_urls;
                        }

                        // Set insert index after existing group tabs
                        if let Some(max_idx) =
                            win.get("tabs").and_then(Value::as_array).and_then(|tabs| {
                                tabs.iter()
                                    .filter(|t| {
                                        t.get("groupId").and_then(Value::as_i64) == Some(gid)
                                    })
                                    .filter_map(|t| t.get("index").and_then(Value::as_i64))
                                    .max()
                            })
                        {
                            self.state.insert_index = Some(max_idx + 1);
                        }
                    }
                }
            }
        }

        self.next_tab_create()
    }

    fn next_tab_create(&mut self) -> OrchStep {
        if self.state.url_idx >= self.state.urls.len() {
            // No more URLs to create
            return self.after_all_tabs_created();
        }

        let url = &self.state.urls[self.state.url_idx];
        let window_id = self.state.window_id.unwrap();

        let mut create_params = serde_json::json!({
            "windowId": window_id,
            "url": url,
            "active": false,
        });

        if let Some(idx) = self.state.insert_index {
            let adjusted = idx + self.state.url_idx as i64;
            create_params
                .as_object_mut()
                .unwrap()
                .insert("index".to_string(), Value::Number(adjusted.into()));
        }

        self.phase = OpenPhase::CreateTabs;
        OrchStep::SendPrimitive {
            action: "p:tab-create".to_string(),
            params: create_params,
        }
    }

    fn handle_tab_created(&mut self, response: Value) -> OrchStep {
        let tab_id = response.get("id").and_then(Value::as_i64);
        if let Some(tid) = tab_id {
            self.state.created.push(serde_json::json!({
                "tabId": tid,
                "windowId": response.get("windowId"),
                "index": response.get("index"),
                "url": response.get("url"),
                "title": response.get("title"),
            }));
        } else {
            let url = self.state.urls.get(self.state.url_idx).cloned();
            self.state
                .skipped
                .push(serde_json::json!({"url": url, "reason": "create_failed"}));
        }

        self.state.url_idx += 1;
        self.next_tab_create()
    }

    fn after_all_tabs_created(&mut self) -> OrchStep {
        // Remove seed tab if new window and we created tabs
        if self.state.seed_tab_id.is_some()
            && !self.state.created.is_empty()
            && !self.state.urls.is_empty()
        {
            self.phase = OpenPhase::RemoveSeedTab;
            return OrchStep::SendPrimitive {
                action: "p:tab-remove".to_string(),
                params: serde_json::json!({"tabIds": [self.state.seed_tab_id]}),
            };
        }

        self.try_group_tabs()
    }

    fn handle_seed_removed(&mut self) -> OrchStep {
        self.try_group_tabs()
    }

    fn try_group_tabs(&mut self) -> OrchStep {
        if self.state.group_title.is_none() || self.state.created.is_empty() {
            return self.try_verify_grouping();
        }

        let tab_ids: Vec<i64> = self
            .state
            .created
            .iter()
            .filter_map(|c| c.get("tabId").and_then(Value::as_i64))
            .collect();

        if tab_ids.is_empty() {
            return self.complete();
        }

        self.phase = OpenPhase::GroupTabs;

        if let Some(gid) = self.state.existing_group_id {
            // Reuse existing group
            self.state.new_group_id = Some(gid);
            self.state.need_group_update = false;
            OrchStep::SendPrimitive {
                action: "p:tab-group".to_string(),
                params: serde_json::json!({"groupId": gid, "tabIds": tab_ids}),
            }
        } else {
            // Create new group
            let window_id = self.state.window_id.unwrap();
            self.state.need_group_update = true;
            OrchStep::SendPrimitive {
                action: "p:tab-group".to_string(),
                params: serde_json::json!({
                    "tabIds": tab_ids,
                    "createProperties": {"windowId": window_id}
                }),
            }
        }
    }

    fn handle_grouped(&mut self, response: Value) -> OrchStep {
        let group_id = response
            .get("groupId")
            .and_then(Value::as_i64)
            .or_else(|| response.as_i64());

        if let Some(gid) = group_id {
            self.state.new_group_id = Some(gid);
        }

        if self.state.need_group_update {
            if let Some(gid) = self.state.new_group_id {
                let mut update = Map::new();
                if let Some(title) = &self.state.group_title {
                    update.insert("title".to_string(), Value::String(title.clone()));
                }
                if let Some(color) = &self.state.group_color {
                    update.insert("color".to_string(), Value::String(color.clone()));
                }
                if !update.is_empty() {
                    self.phase = OpenPhase::UpdateGroup;
                    update.insert("groupId".to_string(), serde_json::json!(gid));
                    return OrchStep::SendPrimitive {
                        action: "p:group-update".to_string(),
                        params: Value::Object(update),
                    };
                }
            }
        }

        self.try_verify_grouping()
    }

    fn handle_group_updated(&mut self) -> OrchStep {
        self.try_verify_grouping()
    }

    fn try_verify_grouping(&mut self) -> OrchStep {
        let target_group_id = self.state.new_group_id.or(self.state.existing_group_id);
        if target_group_id.is_none() || self.state.created.is_empty() {
            return self.complete();
        }

        // Query tabs to verify grouping
        let window_id = self.state.window_id.unwrap();
        self.phase = OpenPhase::VerifyQuery;
        OrchStep::SendPrimitive {
            action: "p:tab-query".to_string(),
            params: serde_json::json!({"query": {"windowId": window_id}}),
        }
    }

    fn handle_verify_query(&mut self, response: Value) -> OrchStep {
        let target_group_id = self.state.new_group_id.or(self.state.existing_group_id);
        let Some(target_gid) = target_group_id else {
            return self.complete();
        };

        let created_ids: std::collections::HashSet<i64> = self
            .state
            .created
            .iter()
            .filter_map(|c| c.get("tabId").and_then(Value::as_i64))
            .collect();

        // Find tabs that should be grouped but aren't
        let tabs = response.as_array().unwrap_or(&Vec::new()).clone();
        let straggler_ids: Vec<i64> = tabs
            .iter()
            .filter(|tab| {
                let tid = tab.get("id").and_then(Value::as_i64).unwrap_or(0);
                let gid = tab.get("groupId").and_then(Value::as_i64).unwrap_or(-1);
                created_ids.contains(&tid) && gid != target_gid
            })
            .filter_map(|tab| tab.get("id").and_then(Value::as_i64))
            .collect();

        if !straggler_ids.is_empty() {
            self.phase = OpenPhase::VerifyGroup;
            return OrchStep::SendPrimitive {
                action: "p:tab-group".to_string(),
                params: serde_json::json!({"groupId": target_gid, "tabIds": straggler_ids}),
            };
        }

        self.complete()
    }

    fn complete(&self) -> OrchStep {
        let group_id = self.state.new_group_id.or(self.state.existing_group_id);
        let created_tab_ids: Vec<Value> = self
            .state
            .created
            .iter()
            .filter_map(|c| c.get("tabId").cloned())
            .collect();

        OrchStep::Complete {
            response: serde_json::json!({
                "windowId": self.state.window_id,
                "groupId": group_id,
                "created": self.state.created,
                "createdTabIds": created_tab_ids,
                "skipped": self.state.skipped,
                "summary": {
                    "createdTabs": self.state.created.len(),
                    "skippedUrls": self.state.skipped.len(),
                    "grouped": group_id.is_some(),
                },
            }),
            undo: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host_impl::orchestrate::Orchestration;

    #[test]
    fn open_new_window_creates_tabs_and_groups() {
        let params = serde_json::json!({
            "urls": ["https://a.com", "https://b.com"],
            "newWindow": true,
            "groupTitle": "Test",
            "color": "blue"
        });
        let mut orch = OpenOrchestration::new(&params);

        // start: create window
        let step = orch.start();
        assert!(
            matches!(&step, OrchStep::SendPrimitive { action, .. } if action == "p:window-create")
        );

        // window created → create first tab
        let step = orch.step(serde_json::json!({
            "id": 100,
            "tabs": [{"id": 999}]
        }));
        assert!(matches!(&step, OrchStep::SendPrimitive { action, params }
                if action == "p:tab-create" && params["url"] == "https://a.com"));

        // first tab created → create second tab
        let step = orch.step(serde_json::json!({"id": 1, "windowId": 100, "index": 0}));
        assert!(matches!(&step, OrchStep::SendPrimitive { action, params }
                if action == "p:tab-create" && params["url"] == "https://b.com"));

        // second tab created → remove seed tab
        let step = orch.step(serde_json::json!({"id": 2, "windowId": 100, "index": 1}));
        assert!(
            matches!(&step, OrchStep::SendPrimitive { action, .. } if action == "p:tab-remove")
        );

        // seed removed → group tabs
        let step = orch.step(serde_json::json!({"removed": true}));
        assert!(matches!(&step, OrchStep::SendPrimitive { action, .. } if action == "p:tab-group"));

        // grouped → update group
        let step = orch.step(serde_json::json!({"groupId": 50}));
        assert!(matches!(&step, OrchStep::SendPrimitive { action, params }
                if action == "p:group-update" && params["title"] == "Test"));

        // updated → verify query
        let step = orch.step(serde_json::json!({"id": 50}));
        assert!(matches!(&step, OrchStep::SendPrimitive { action, .. } if action == "p:tab-query"));

        // all grouped → complete
        let step = orch.step(serde_json::json!([
            {"id": 1, "groupId": 50},
            {"id": 2, "groupId": 50}
        ]));
        let OrchStep::Complete { response, .. } = step else {
            panic!("expected Complete");
        };
        assert_eq!(response["windowId"], 100);
        assert_eq!(response["summary"]["createdTabs"], 2);
        assert_eq!(response["summary"]["grouped"], true);
    }

    #[test]
    fn open_existing_window_deduplicates_urls() {
        let params = serde_json::json!({
            "urls": ["https://a.com", "https://b.com"],
            "groupTitle": "Dev",
            "windowId": 100
        });
        let mut orch = OpenOrchestration::new(&params);

        let step = orch.start();
        assert!(matches!(&step, OrchStep::SendPrimitive { action, .. } if action == "p:snapshot"));

        let snapshot = serde_json::json!({
            "windows": [{
                "windowId": 100,
                "focused": true,
                "tabs": [
                    {"tabId": 1, "windowId": 100, "index": 0, "groupId": 10, "url": "https://a.com"}
                ],
                "groups": [
                    {"groupId": 10, "title": "Dev", "color": "blue", "collapsed": false}
                ]
            }]
        });

        // Snapshot received — a.com is duplicate, only b.com should be created
        let step = orch.step(snapshot);
        let OrchStep::SendPrimitive { action, params } = &step else {
            panic!("expected tab-create, got {step:?}");
        };
        assert_eq!(action, "p:tab-create");
        assert_eq!(params["url"], "https://b.com");
    }

    #[test]
    fn open_no_urls_errors() {
        let params = serde_json::json!({});
        let mut orch = OpenOrchestration::new(&params);
        let step = orch.start();
        assert!(matches!(&step, OrchStep::Error { message, .. } if message.contains("No URLs")));
    }

    #[test]
    fn normalize_url_strips_protocol_and_trailing_slash() {
        assert_eq!(normalize_url("https://example.com/"), "example.com");
        assert_eq!(
            normalize_url("http://Example.COM/path#frag"),
            "example.com/path"
        );
    }

    #[test]
    fn open_new_window_no_urls_creates_empty_window() {
        let params = serde_json::json!({"newWindow": true});
        let mut orch = OpenOrchestration::new(&params);

        let step = orch.start();
        assert!(
            matches!(&step, OrchStep::SendPrimitive { action, .. } if action == "p:window-create")
        );

        // Window created, no URLs → complete
        let step = orch.step(serde_json::json!({"id": 100, "tabs": [{"id": 999}]}));
        let OrchStep::Complete { response, .. } = step else {
            panic!("expected Complete, got {step:?}");
        };
        assert_eq!(response["windowId"], 100);
        assert_eq!(response["summary"]["createdTabs"], 0);
    }
}
