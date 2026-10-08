//! Host response contracts that must not turn missing metadata into successful results.

use crate::context::GqlContext;
use crate::convert::{tab_from_value, windows_from_snapshot};
use crate::types::{CloseResult, Group, PingResult, SkippedTab, Tab, UndoResult};
use juniper::{FieldError, FieldResult};
use serde_json::Value;

fn invalid(message: &str) -> FieldError {
    FieldError::new(message, juniper::Value::Null)
}

fn required_string(response: &Value, field: &str) -> FieldResult<String> {
    response
        .get(field)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .map(String::from)
        .ok_or_else(|| invalid(&format!("Host response is missing {field}")))
}

pub(crate) fn live_group_result(ctx: &GqlContext, group_id: i32) -> FieldResult<Group> {
    let snapshot = ctx.sender.snapshot().map_err(|error| {
        invalid(&format!(
            "Group mutation completed (groupId: {group_id}) but result snapshot failed: {error}"
        ))
    })?;
    windows_from_snapshot(&snapshot)
        .into_iter()
        .flat_map(|window| window.groups)
        .find(|group| group.group_id == group_id)
        .ok_or_else(|| {
            invalid(&format!(
                "Mutated group {group_id} is absent from the live result snapshot"
            ))
        })
}

pub(crate) fn ping_from_response(response: &Value, latency_ms: f64) -> FieldResult<PingResult> {
    if response
        .get("nativeChannelAvailable")
        .and_then(Value::as_bool)
        != Some(true)
    {
        return Err(invalid("Native browser round trip is unavailable"));
    }
    let optional = |key| response.get(key).and_then(Value::as_str).map(String::from);
    Ok(PingResult {
        ok: true,
        latency_ms,
        runtime_id: required_string(response, "runtimeId")?,
        version: required_string(response, "version")?,
        base_version: optional("baseVersion"),
        git_sha: optional("gitSha"),
        dirty: response.get("dirty").and_then(Value::as_bool),
        host_version: optional("hostVersion"),
        host_base_version: optional("hostBaseVersion"),
        host_git_sha: optional("hostGitSha"),
        host_dirty: response.get("hostDirty").and_then(Value::as_bool),
        versions_in_sync: response.get("versionsInSync").and_then(Value::as_bool),
        native_channel_available: true,
    })
}

pub(crate) fn undo_metadata(
    response: &Value,
    mutated: bool,
) -> FieldResult<(Option<String>, Option<String>)> {
    let txid = match response.get("txid") {
        None | Some(Value::Null) => None,
        Some(Value::String(txid)) if !txid.trim().is_empty() => Some(txid.clone()),
        Some(_) => {
            return Err(invalid(
                "Host response has an invalid undo transaction identifier",
            ))
        }
    };
    let unavailable = match response.get("undoUnavailable") {
        None | Some(Value::Null) => None,
        Some(Value::String(reason)) if !reason.trim().is_empty() => Some(reason.clone()),
        Some(_) => {
            return Err(invalid(
                "Host response has an invalid undoUnavailable reason",
            ))
        }
    };
    if mutated && txid.is_none() && unavailable.is_none() {
        return Err(invalid(
            "Mutation completed but host response is missing the undo transaction identifier",
        ));
    }
    Ok((txid, unavailable))
}

pub(crate) fn skipped_tabs(response: &Value) -> FieldResult<Vec<SkippedTab>> {
    let items = match response.get("skipped") {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Array(items)) => items,
        Some(_) => return Err(invalid("Host response has an invalid skipped tab list")),
    };
    items
        .iter()
        .map(|item| {
            Ok(SkippedTab {
                tab_id: item
                    .get("tabId")
                    .and_then(Value::as_i64)
                    .ok_or_else(|| invalid("Skipped result is missing tabId"))?
                    as i32,
                reason: required_string(item, "reason")?,
            })
        })
        .collect()
}

pub(crate) fn close_from_response(
    response: &Value,
    remaining_tabs: Vec<Tab>,
) -> FieldResult<CloseResult> {
    let summary = response
        .get("summary")
        .ok_or_else(|| invalid("Close response is missing summary"))?;
    let closed_tabs = summary
        .get("closedTabs")
        .and_then(Value::as_i64)
        .ok_or_else(|| invalid("Close response is missing closedTabs"))?
        as i32;
    let dry_run = response
        .get("dryRun")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let (txid, undo_unavailable) = undo_metadata(response, !dry_run && closed_tabs > 0)?;
    let skipped = skipped_tabs(response)?;
    let tabs = response
        .get("tabs")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    tab_from_value(
                        item,
                        item.get("windowId").and_then(Value::as_i64).unwrap_or(0) as i32,
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(CloseResult {
        txid,
        undo_unavailable,
        closed_tabs,
        dry_run,
        planned_tabs: summary
            .get("plannedTabs")
            .and_then(Value::as_i64)
            .unwrap_or(i64::from(closed_tabs)) as i32,
        skipped_tabs: summary
            .get("skippedTabs")
            .and_then(Value::as_i64)
            .unwrap_or(skipped.len() as i64) as i32,
        skipped,
        tabs,
        remaining_tabs,
    })
}

pub(crate) fn undo_from_response(response: &Value) -> FieldResult<UndoResult> {
    Ok(UndoResult {
        txid: required_string(response, "txid")?,
        summary: response
            .get("summary")
            .map(|summary| {
                summary
                    .as_str()
                    .map(String::from)
                    .unwrap_or_else(|| summary.to_string())
            })
            .unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn close_preview_retains_plan_and_policy_skips_without_a_fake_transaction() {
        let parsed = close_from_response(
            &json!({
                "txid": null,
                "dryRun": true,
                "summary": { "closedTabs": 0, "plannedTabs": 1, "skippedTabs": 1 },
                "tabs": [{ "tabId": 8, "windowId": 2, "url": "https://test/" }],
                "skipped": [{ "tabId": 9, "reason": "protected_pinned" }]
            }),
            Vec::new(),
        )
        .unwrap();
        assert_eq!(parsed.txid, None);
        assert_eq!(parsed.undo_unavailable, None);
        assert!(parsed.dry_run);
        assert_eq!(parsed.planned_tabs, 1);
        assert_eq!(parsed.skipped_tabs, 1);
        assert_eq!(parsed.tabs[0].tab_id, 8);
        assert_eq!(parsed.skipped[0].reason, "protected_pinned");
    }

    #[test]
    fn close_preserves_success_when_undo_is_explicitly_unavailable() {
        let result = close_from_response(
            &json!({
                "txid": null,
                "undoUnavailable": "private tabs are never persisted",
                "summary": { "closedTabs": 1 }
            }),
            Vec::new(),
        )
        .expect("a completed private close is not a failed mutation");
        assert_eq!(result.closed_tabs, 1);
        assert_eq!(result.txid, None);
        assert_eq!(
            result.undo_unavailable.as_deref(),
            Some("private tabs are never persisted")
        );
        assert!(!result.dry_run);
    }

    #[test]
    fn close_still_rejects_missing_recovery_metadata() {
        for reason in [Value::Null, json!(""), json!("   "), json!(false)] {
            assert!(close_from_response(
                &json!({
                    "txid": null, "undoUnavailable": reason,
                    "summary": { "closedTabs": 1 }
                }),
                Vec::new(),
            )
            .is_err());
        }
        assert!(close_from_response(&json!({"summary": {"closedTabs": 1}}), Vec::new(),).is_err());
    }

    #[test]
    fn undo_requires_the_actual_transaction_identifier() {
        assert!(undo_from_response(&json!({ "summary": "undone" })).is_err());
        assert_eq!(
            undo_from_response(&json!({ "txid": "tx-real", "summary": "undone" }))
                .unwrap()
                .txid,
            "tx-real"
        );
    }

    #[test]
    fn ping_rejects_host_only_success_and_preserves_browser_versions() {
        assert!(ping_from_response(&json!({ "nativeChannelAvailable": true }), 1.0).is_err());
        let ping = ping_from_response(
            &json!({
                "runtimeId": "extension-id", "version": "1.0", "hostVersion": "2.0",
                "nativeChannelAvailable": true
            }),
            2.5,
        )
        .unwrap();
        assert_eq!(ping.runtime_id, "extension-id");
        assert_eq!(ping.version, "1.0");
        assert_eq!(ping.host_version.as_deref(), Some("2.0"));
        assert_eq!(ping.latency_ms, 2.5);
    }
}
