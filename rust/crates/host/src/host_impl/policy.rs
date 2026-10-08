use serde::Deserialize;
use serde_json::Value;
use std::path::{Path, PathBuf};

use super::orchestrate::scope::{select_tabs_by_scope, ScopedTab};

#[derive(Debug, Default, Deserialize)]
pub(super) struct Policy {
    #[serde(default)]
    protect: Protection,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Protection {
    #[serde(default)]
    pinned: bool,
    #[serde(default)]
    group_titles: Vec<String>,
    #[serde(default)]
    domains: Vec<String>,
}

impl Policy {
    pub(super) fn load(path: &Path) -> Result<Self, String> {
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|err| format!("Invalid protection policy {}: {err}", path.display())),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(err) => Err(format!(
                "Cannot read protection policy {}: {err}",
                path.display()
            )),
        }
    }

    pub(super) fn reason(&self, tab: &ScopedTab) -> Option<&'static str> {
        if self.protect.pinned && tab.pinned == Some(true) {
            return Some("protected_pinned");
        }
        if tab.group_title.as_deref().is_some_and(|title| {
            self.protect
                .group_titles
                .iter()
                .any(|marker| !marker.is_empty() && title.contains(marker))
        }) {
            return Some("protected_group");
        }
        if tab.url.as_deref().and_then(domain).is_some_and(|host| {
            self.protect.domains.iter().any(|protected| {
                let protected = protected
                    .trim()
                    .trim_start_matches("*.")
                    .to_ascii_lowercase();
                !protected.is_empty()
                    && (host == protected || host.ends_with(&format!(".{protected}")))
            })
        }) {
            return Some("protected_domain");
        }
        None
    }

    pub(super) fn check_primitive(
        &self,
        snapshot: &Value,
        action: &str,
        params: &Value,
    ) -> Result<(), String> {
        let ids = affected_tab_ids(snapshot, action, params);
        let tabs = select_tabs_by_scope(snapshot, &serde_json::json!({"tabIds": ids})).tabs;
        let protected: Vec<_> = tabs
            .iter()
            .filter_map(|tab| {
                self.reason(tab)
                    .map(|reason| serde_json::json!({"tabId": tab.tab_id, "reason": reason}))
            })
            .collect();
        if protected.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "Mutation blocked by protection policy: {}",
                serde_json::json!(protected)
            ))
        }
    }
}

pub(super) fn default_policy_path() -> PathBuf {
    let base = std::env::var_os("TABCTL_CONFIG_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::var_os("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    std::env::var_os("HOME")
                        .map(PathBuf::from)
                        .unwrap_or_else(|| PathBuf::from("."))
                        .join(".config")
                })
                .join("tabctl")
        });
    base.join("policy.json")
}

pub(super) fn affected_tab_ids(snapshot: &Value, action: &str, params: &Value) -> Vec<i64> {
    let mut ids: Vec<i64> = params
        .get("tabIds")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_i64)
        .collect();
    if let Some(id) = params.get("tabId").and_then(Value::as_i64) {
        ids.push(id);
    }
    if let Some(group_id) = params.get("groupId").and_then(Value::as_i64) {
        ids.extend(
            select_tabs_by_scope(snapshot, &serde_json::json!({"groupId": group_id}))
                .tabs
                .iter()
                .map(|tab| tab.tab_id),
        );
    }
    if action == "p:window-remove" {
        if let Some(window_id) = params.get("windowId").and_then(Value::as_i64) {
            ids.extend(
                select_tabs_by_scope(snapshot, &serde_json::json!({"windowId": window_id}))
                    .tabs
                    .iter()
                    .map(|tab| tab.tab_id),
            );
        }
    }
    ids.sort_unstable();
    ids.dedup();
    ids
}

fn domain(url: &str) -> Option<String> {
    let (_, rest) = url.split_once("://")?;
    let authority = rest.split(['/', '?', '#']).next()?.rsplit('@').next()?;
    Some(
        authority
            .split(':')
            .next()?
            .trim_end_matches('.')
            .to_ascii_lowercase(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protection_matches_pins_group_markers_and_domain_boundaries() {
        let policy: Policy = serde_json::from_value(serde_json::json!({
            "protect": {"pinned": true, "groupTitles": ["🔒"], "domains": ["example.com"]}
        }))
        .unwrap();
        let snapshot = serde_json::json!({"windows": [{"windowId": 1, "tabs": [
            {"tabId": 1, "pinned": true},
            {"tabId": 2, "groupTitle": "Work 🔒"},
            {"tabId": 3, "url": "https://sub.example.com/path"},
            {"tabId": 4, "url": "https://notexample.com"},
            {"tabId": 5, "url": "https://example.com.evil.invalid"}
        ]}]});
        let tabs = select_tabs_by_scope(&snapshot, &serde_json::json!({"all": true})).tabs;
        let reasons: Vec<_> = tabs.iter().map(|tab| policy.reason(tab)).collect();
        assert_eq!(
            reasons,
            vec![
                Some("protected_pinned"),
                Some("protected_group"),
                Some("protected_domain"),
                None,
                None
            ]
        );
    }
}
