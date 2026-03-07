use serde_json::{Map, Value};
use std::collections::HashMap;
use tabctl_shared::normalize_url;

use super::scope::{select_tabs_by_scope, ScopedTab};
use super::OrchStep;

const DEFAULT_STALE_DAYS: u64 = 30;

/// Orchestration for the `analyze` command.
///
/// p:snapshot → pure analysis (staleness, duplicates, domain stats) →
/// if dedupe confirmed: p:tab-remove → respond with undo.
#[derive(Debug)]
pub(crate) struct AnalyzeOrchestration {
    params: Value,
    phase: AnalyzePhase,
    state: Option<AnalyzeState>,
}

#[derive(Debug)]
struct AnalyzeState {
    analysis: Value,
    dedupe_tab_ids: Vec<i64>,
    dedupe_undo_tabs: Vec<Value>,
    has_incognito: bool,
}

#[derive(Debug)]
enum AnalyzePhase {
    GetSnapshot,
    DedupeRemove,
}

impl AnalyzeOrchestration {
    pub(crate) fn new(params: &Value) -> Self {
        Self {
            params: params.clone(),
            phase: AnalyzePhase::GetSnapshot,
            state: None,
        }
    }
}

impl super::Orchestration for AnalyzeOrchestration {
    fn start(&mut self) -> OrchStep {
        OrchStep::SendPrimitive {
            action: "p:snapshot".to_string(),
            params: Value::Object(Map::new()),
        }
    }

    fn step(&mut self, response: Value) -> OrchStep {
        match self.phase {
            AnalyzePhase::GetSnapshot => self.handle_snapshot(response),
            AnalyzePhase::DedupeRemove => self.handle_dedupe_complete(),
        }
    }
}

impl AnalyzeOrchestration {
    fn handle_snapshot(&mut self, snapshot: Value) -> OrchStep {
        let scope_result = select_tabs_by_scope(&snapshot, &self.params);
        if let Some(err) = scope_result.error {
            return OrchStep::Error {
                message: err,
                hint: None,
            };
        }

        let tabs = scope_result.tabs;
        let now_ms = now_ms();
        let stale_days = self
            .params
            .get("staleDays")
            .and_then(Value::as_u64)
            .unwrap_or(DEFAULT_STALE_DAYS);
        let stale_threshold_ms = stale_days * 86_400_000;

        // Build tab values and compute staleness
        let mut tab_values: Vec<Value> = Vec::new();
        let mut stale_tabs: Vec<Value> = Vec::new();

        for tab in &tabs {
            let tab_val = tab_to_value(tab);
            tab_values.push(tab_val.clone());

            if let Some(lfa) = tab.last_accessed_at {
                if now_ms.saturating_sub(lfa as u64) > stale_threshold_ms {
                    stale_tabs.push(tab_val);
                }
            }
        }

        // Find duplicates by normalized URL
        let mut url_groups: HashMap<String, Vec<&ScopedTab>> = HashMap::new();
        for tab in &tabs {
            if let Some(url) = tab.url.as_deref() {
                let normalized = normalize_url(url);
                url_groups.entry(normalized).or_default().push(tab);
            }
        }

        let mut duplicates: Vec<Value> = Vec::new();
        for (normalized, group) in &url_groups {
            if group.len() > 1 {
                let group_tabs: Vec<Value> = group.iter().map(|t| tab_to_value(t)).collect();
                duplicates.push(serde_json::json!({
                    "normalizedUrl": normalized,
                    "tabs": group_tabs,
                }));
            }
        }

        // Domain frequency
        let mut domain_counts: HashMap<String, usize> = HashMap::new();
        for tab in &tabs {
            if let Some(domain) = tab.url.as_deref().and_then(extract_domain) {
                *domain_counts.entry(domain).or_insert(0) += 1;
            }
        }

        let unique_domain_count = domain_counts.len();
        let domains: Map<String, Value> = domain_counts
            .into_iter()
            .map(|(k, v)| (k, Value::Number(v.into())))
            .collect();

        // Candidates for close --apply (stale tabs)
        let candidates: Vec<Value> = stale_tabs
            .iter()
            .map(|t| {
                serde_json::json!({
                    "tabId": t["tabId"],
                    "url": t["url"],
                })
            })
            .collect();

        let analysis = serde_json::json!({
            "tabs": tab_values,
            "stale": stale_tabs,
            "duplicates": duplicates,
            "domains": Value::Object(domains),
            "summary": {
                "totalTabs": tabs.len(),
                "staleTabs": stale_tabs.len(),
                "duplicateGroups": duplicates.len(),
                "uniqueDomains": unique_domain_count,
            },
            "candidates": candidates,
        });

        // Check for dedupe mode
        let dedupe = self
            .params
            .get("dedupe")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let confirm = self
            .params
            .get("confirmed")
            .or_else(|| self.params.get("confirm"))
            .and_then(Value::as_bool)
            .unwrap_or(false);

        if dedupe && confirm {
            // Close duplicate tabs, keeping the first in each group
            let mut remove_ids: Vec<i64> = Vec::new();
            let mut undo_tabs: Vec<Value> = Vec::new();

            for group in url_groups.values() {
                if group.len() > 1 {
                    for tab in group.iter().skip(1) {
                        remove_ids.push(tab.tab_id);
                        undo_tabs.push(serde_json::json!({
                            "url": tab.url,
                            "title": tab.title,
                            "pinned": tab.pinned,
                            "active": tab.active,
                            "from": {
                                "windowId": tab.window_id,
                                "index": tab.index,
                                "groupId": tab.group_id,
                                "groupTitle": tab.group_title,
                                "groupColor": tab.group_color,
                                "groupCollapsed": tab.group_collapsed,
                            }
                        }));
                    }
                }
            }

            if remove_ids.is_empty() {
                return OrchStep::Complete {
                    response: analysis,
                    undo: None,
                };
            }

            self.state = Some(AnalyzeState {
                analysis,
                dedupe_tab_ids: remove_ids.clone(),
                dedupe_undo_tabs: undo_tabs,
                has_incognito: url_groups
                    .values()
                    .any(|group| group.iter().skip(1).any(|tab| tab.incognito)),
            });
            self.phase = AnalyzePhase::DedupeRemove;

            return OrchStep::SendPrimitive {
                action: "p:tab-remove".to_string(),
                params: serde_json::json!({ "tabIds": remove_ids }),
            };
        }

        OrchStep::Complete {
            response: analysis,
            undo: None,
        }
    }

    fn handle_dedupe_complete(&self) -> OrchStep {
        let state = self.state.as_ref().unwrap();
        let mut analysis = state.analysis.as_object().cloned().unwrap_or_else(Map::new);

        analysis.insert(
            "dedupeSummary".to_string(),
            serde_json::json!({
                "closedTabs": state.dedupe_tab_ids.len(),
            }),
        );

        OrchStep::Complete {
            response: Value::Object(analysis),
            undo: Some(serde_json::json!({
                "action": "close",
                "incognito": state.has_incognito,
                "tabs": state.dedupe_undo_tabs,
            })),
        }
    }
}

fn tab_to_value(tab: &ScopedTab) -> Value {
    serde_json::json!({
        "tabId": tab.tab_id,
        "windowId": tab.window_id,
        "url": tab.url,
        "title": tab.title,
        "groupId": tab.group_id,
        "groupTitle": tab.group_title,
        "active": tab.active,
        "pinned": tab.pinned,
        "lastAccessedAt": tab.last_accessed_at,
    })
}

fn extract_domain(url: &str) -> Option<String> {
    let stripped = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .unwrap_or(url);
    let domain = stripped.split('/').next()?;
    if domain.is_empty() {
        return None;
    }
    Some(domain.to_lowercase())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host_impl::orchestrate::Orchestration;

    fn snapshot_with(tabs: Vec<Value>) -> Value {
        serde_json::json!({
            "windows": [{
                "windowId": 100,
                "focused": true,
                "tabs": tabs,
                "groups": []
            }]
        })
    }

    #[test]
    fn analyze_basic() {
        let params = serde_json::json!({});
        let mut orch = AnalyzeOrchestration::new(&params);
        let _ = orch.start();

        let snap = snapshot_with(vec![
            serde_json::json!({"tabId": 1, "windowId": 100, "url": "https://a.com", "title": "A", "groupId": -1}),
            serde_json::json!({"tabId": 2, "windowId": 100, "url": "https://b.com", "title": "B", "groupId": -1}),
        ]);

        let step = orch.step(snap);
        let OrchStep::Complete { response, .. } = step else {
            panic!("expected Complete");
        };
        assert_eq!(response["summary"]["totalTabs"], 2);
        assert_eq!(response["summary"]["staleTabs"], 0);
        assert!(response["tabs"].as_array().unwrap().len() == 2);
    }

    #[test]
    fn analyze_finds_duplicates() {
        let params = serde_json::json!({});
        let mut orch = AnalyzeOrchestration::new(&params);
        let _ = orch.start();

        let snap = snapshot_with(vec![
            serde_json::json!({"tabId": 1, "windowId": 100, "url": "https://a.com", "title": "A", "groupId": -1}),
            serde_json::json!({"tabId": 2, "windowId": 100, "url": "https://a.com", "title": "A dup", "groupId": -1}),
            serde_json::json!({"tabId": 3, "windowId": 100, "url": "https://b.com", "title": "B", "groupId": -1}),
        ]);

        let step = orch.step(snap);
        let OrchStep::Complete { response, .. } = step else {
            panic!("expected Complete");
        };
        assert_eq!(response["summary"]["duplicateGroups"], 1);
        let dups = response["duplicates"].as_array().unwrap();
        assert_eq!(dups.len(), 1);
        assert_eq!(dups[0]["tabs"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn analyze_dedupe_closes_duplicates() {
        let params = serde_json::json!({"dedupe": true, "confirm": true});
        let mut orch = AnalyzeOrchestration::new(&params);
        let _ = orch.start();

        let snap = snapshot_with(vec![
            serde_json::json!({"tabId": 1, "windowId": 100, "index": 0, "url": "https://a.com", "title": "A", "groupId": -1}),
            serde_json::json!({"tabId": 2, "windowId": 100, "index": 1, "url": "https://a.com", "title": "A dup", "groupId": -1}),
        ]);

        // Snapshot → p:tab-remove for duplicate (tab 2)
        let step = orch.step(snap);
        let OrchStep::SendPrimitive { action, params } = &step else {
            panic!("expected SendPrimitive, got {step:?}");
        };
        assert_eq!(action, "p:tab-remove");
        let remove_ids = params["tabIds"].as_array().unwrap();
        assert_eq!(remove_ids.len(), 1);
        assert_eq!(remove_ids[0], 2);

        // Remove complete → Complete with undo
        let step = orch.step(serde_json::json!({"removed": true}));
        let OrchStep::Complete { response, undo } = step else {
            panic!("expected Complete");
        };
        assert_eq!(response["dedupeSummary"]["closedTabs"], 1);
        let undo = undo.unwrap();
        assert_eq!(undo["action"], "close");
        assert_eq!(undo["tabs"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn normalize_url_strips_protocol_www_trailing_slash() {
        assert_eq!(normalize_url("https://www.example.com/"), "example.com");
        assert_eq!(
            normalize_url("http://example.com/path?b=2&a=1"),
            "example.com/path?a=1&b=2"
        );
        assert_eq!(
            normalize_url("https://example.com/page#section"),
            "example.com/page"
        );
    }
}
