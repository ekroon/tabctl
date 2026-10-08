use crate::host_impl::policy::Policy;
use serde_json::{Map, Value};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tabctl_shared::normalize_url;

use super::scope::{select_tabs_by_scope, ScopedTab};
use super::OrchStep;

const DEFAULT_STALE_DAYS: u64 = 30;

/// Orchestration for the `analyze` command.
///
/// p:snapshot → pure analysis (staleness, duplicates, domain stats) →
/// when dedupe is requested: build a protection-aware plan, then remove only
/// when confirmed and not dry-run. Recovery belongs to the host boundary.
#[derive(Debug)]
pub(crate) struct AnalyzeOrchestration {
    params: Value,
    phase: AnalyzePhase,
    state: Option<AnalyzeState>,
    policy: Arc<Policy>,
}

#[derive(Debug)]
struct AnalyzeState {
    analysis: Value,
    dedupe_tab_ids: Vec<i64>,
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
            policy: Arc::new(Policy::default()),
        }
    }
}

impl super::Orchestration for AnalyzeOrchestration {
    fn set_policy(&mut self, policy: Arc<Policy>) {
        self.policy = policy;
    }
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
        let tab_values = scoped_tab_values(&snapshot, &tabs);
        let tab_index: HashMap<_, _> = tabs
            .iter()
            .zip(&tab_values)
            .map(|(tab, value)| (tab.tab_id, value))
            .collect();
        let mut stale_tabs: Vec<Value> = Vec::new();

        for (tab, tab_val) in tabs.iter().zip(&tab_values) {
            if let Some(lfa) = tab.last_accessed_at {
                if now_ms.saturating_sub(lfa as u64) > stale_threshold_ms {
                    stale_tabs.push(tab_val.clone());
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
                let group_tabs: Vec<Value> = group
                    .iter()
                    .map(|tab| tab_index[&tab.tab_id].clone())
                    .collect();
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

        let mut analysis = serde_json::json!({
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

        if dedupe {
            // Close duplicate tabs, keeping the first in each group
            let mut remove_ids: Vec<i64> = Vec::new();
            let mut skipped = Vec::new();
            let duplicate_ids: HashSet<_> = url_groups
                .values()
                .flat_map(|group| group.iter().skip(1).map(|tab| tab.tab_id))
                .collect();
            for tab in tabs
                .iter()
                .filter(|tab| duplicate_ids.contains(&tab.tab_id))
            {
                if let Some(reason) = self.policy.reason(tab) {
                    skipped.push(serde_json::json!({"tabId":tab.tab_id,"reason":reason}));
                    continue;
                }
                remove_ids.push(tab.tab_id);
            }

            analysis["skipped"] = serde_json::json!(skipped);
            analysis["dedupeSummary"] = serde_json::json!({"closedTabs":0,"plannedTabs":remove_ids.len(),"skippedTabs":skipped.len()});
            if !confirm || self.params["dryRun"].as_bool() == Some(true) || remove_ids.is_empty() {
                return OrchStep::Complete {
                    response: analysis,
                    undo: None,
                };
            }

            self.state = Some(AnalyzeState {
                analysis,
                dedupe_tab_ids: remove_ids.clone(),
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
        let mut analysis = state.analysis.clone();
        analysis["dedupeSummary"]["closedTabs"] = serde_json::json!(state.dedupe_tab_ids.len());

        OrchStep::Complete {
            response: analysis,
            undo: None,
        }
    }
}

pub(super) fn scoped_tab_values(snapshot: &Value, tabs: &[ScopedTab]) -> Vec<Value> {
    let mut originals = HashMap::new();
    for window in snapshot["windows"].as_array().into_iter().flatten() {
        let window_id = window["windowId"].as_i64().unwrap_or(0);
        for tab in window["tabs"].as_array().into_iter().flatten() {
            if let Some(id) = tab["tabId"].as_i64() {
                originals.insert((window_id, id), tab);
            }
        }
    }
    tabs.iter()
        .map(|tab| tab_to_value(tab, originals.get(&(tab.window_id, tab.tab_id)).copied()))
        .collect()
}

fn tab_to_value(tab: &ScopedTab, original: Option<&Value>) -> Value {
    let mut fields = original
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    let last_accessed_at = tab
        .last_accessed_at
        .map(Value::from)
        .or_else(|| original.and_then(|tab| tab.get("lastAccessedAt")).cloned())
        .unwrap_or(Value::Null);
    fields.extend(
        serde_json::json!({
            "tabId": tab.tab_id,
            "windowId": tab.window_id,
            "index": tab.index,
            "incognito": tab.incognito,
            "url": tab.url,
            "title": tab.title,
            "groupId": tab.group_id,
            "groupTitle": tab.group_title,
            "groupColor": tab.group_color,
            "groupCollapsed": tab.group_collapsed,
            "active": tab.active,
            "pinned": tab.pinned,
            "lastAccessedAt": last_accessed_at,
        })
        .as_object()
        .unwrap()
        .clone(),
    );
    Value::Object(fields)
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

        // Transaction recovery is owned by the host mutation boundary.
        let step = orch.step(serde_json::json!({"removed": true}));
        let OrchStep::Complete { response, undo } = step else {
            panic!("expected Complete");
        };
        assert_eq!(response["dedupeSummary"]["closedTabs"], 1);
        assert!(undo.is_none());
    }

    fn protected_duplicates() -> Value {
        serde_json::json!({"windows":[{
            "windowId":100,"focused":true,
            "groups":[
                {"groupId":30,"title":"Protected 🔒","color":"red","collapsed":false},
                {"groupId":40,"title":"Work","color":"blue","collapsed":true}
            ],
            "tabs":[
                {"tabId":1,"index":0,"url":"https://a.example","lastAccessedAt":1},
                {"tabId":2,"index":1,"url":"https://a.example","pinned":true},
                {"tabId":3,"index":2,"url":"https://a.example","groupId":30},
                {"tabId":4,"index":8,"url":"https://a.example","title":"Eligible","groupId":40,"active":true,"lastAccessedAt":2,
                    "status":"complete","favIconUrl":"https://a.example/icon.png","audible":true,"discarded":true},
                {"tabId":5,"index":9,"url":"https://blocked.example"},
                {"tabId":6,"index":10,"url":"https://blocked.example"}
            ]
        }]})
    }

    fn dedupe_with_policy(params: Value) -> AnalyzeOrchestration {
        let mut orch = AnalyzeOrchestration::new(&params);
        orch.set_policy(Arc::new(
            serde_json::from_value(serde_json::json!({
                "protect":{"pinned":true,"groupTitles":["🔒"],"domains":["blocked.example"]}
            }))
            .unwrap(),
        ));
        let _ = orch.start();
        orch
    }

    fn assert_dedupe_plan(response: &Value, closed: usize) {
        assert_eq!(
            response["dedupeSummary"],
            serde_json::json!({
                "closedTabs":closed,"plannedTabs":1,"skippedTabs":3
            })
        );
        let candidates: Vec<_> = response["duplicates"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|group| group["tabs"].as_array().unwrap().iter().skip(1))
            .filter(|tab| {
                !response["skipped"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|skip| skip["tabId"] == tab["tabId"])
            })
            .collect();
        assert_eq!(candidates.len(), 1);
        let candidate = candidates[0];
        assert_eq!(candidate["tabId"], 4);
        assert_eq!(candidate["index"], 8);
        assert_eq!(candidate["groupId"], 40);
        assert_eq!(candidate["groupTitle"], "Work");
        assert_eq!(candidate["groupColor"], "blue");
        assert_eq!(candidate["groupCollapsed"], true);
        assert_eq!(candidate["active"], true);
        assert_eq!(candidate["audible"], true);
        assert_eq!(candidate["status"], "complete");
        assert_eq!(
            response["skipped"],
            serde_json::json!([
                {"tabId":2,"reason":"protected_pinned"},
                {"tabId":3,"reason":"protected_group"},
                {"tabId":6,"reason":"protected_domain"}
            ])
        );
        assert_eq!(response["duplicates"].as_array().unwrap().len(), 2);
        let stale_ids: Vec<_> = response["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .map(|tab| tab["tabId"].as_i64().unwrap())
            .collect();
        assert_eq!(
            stale_ids,
            vec![1, 4],
            "stale candidates must keep their original meaning"
        );
    }

    #[test]
    fn dedupe_previews_and_dry_runs_build_the_same_protected_plan_without_removal() {
        for params in [
            serde_json::json!({"dedupe":true}),
            serde_json::json!({"dedupe":true,"dryRun":true}),
            serde_json::json!({"dedupe":true,"confirmed":true,"dryRun":true}),
        ] {
            let mut orch = dedupe_with_policy(params);
            let OrchStep::Complete { response, .. } = orch.step(protected_duplicates()) else {
                panic!("preview must not remove")
            };
            assert_dedupe_plan(&response, 0);
        }
    }

    #[test]
    fn confirmed_dedupe_removes_only_planned_eligible_tabs_and_preserves_plan_projection() {
        let mut orch = dedupe_with_policy(serde_json::json!({"dedupe":true,"confirmed":true}));
        let OrchStep::SendPrimitive { action, params } = orch.step(protected_duplicates()) else {
            panic!("expected eligible removal")
        };
        assert_eq!(action, "p:tab-remove");
        assert_eq!(params["tabIds"], serde_json::json!([4]));
        let OrchStep::Complete { response, .. } = orch.step(serde_json::json!({})) else {
            panic!("expected result")
        };
        assert_dedupe_plan(&response, 1);
    }

    #[test]
    fn all_protected_dedupe_plans_are_explicit_empty_no_ops_in_every_mode() {
        for params in [
            serde_json::json!({"dedupe":true}),
            serde_json::json!({"dedupe":true,"confirmed":true}),
            serde_json::json!({"dedupe":true,"confirmed":true,"dryRun":true}),
        ] {
            let mut snapshot = protected_duplicates();
            snapshot["windows"][0]["tabs"]
                .as_array_mut()
                .unwrap()
                .retain(|tab| tab["tabId"] != 4);
            let mut orch = dedupe_with_policy(params);
            let OrchStep::Complete { response, .. } = orch.step(snapshot) else {
                panic!("must not remove protected tabs")
            };
            assert!(response.get("dedupeCandidates").is_none());
            assert_eq!(
                response["dedupeSummary"],
                serde_json::json!({
                    "closedTabs":0,"plannedTabs":0,"skippedTabs":3
                })
            );
            assert_eq!(response["skipped"].as_array().unwrap().len(), 3);
        }
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
