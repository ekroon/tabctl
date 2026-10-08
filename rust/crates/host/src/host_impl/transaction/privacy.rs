use serde_json::{json, Value};

use crate::host_impl::orchestrate::scope::select_tabs_by_scope;
use crate::host_impl::policy::affected_tab_ids;

#[derive(Debug, Default)]
pub(in crate::host_impl) struct MutationScope {
    pub(in crate::host_impl) tab_ids: Vec<i64>,
    pub(in crate::host_impl) group_ids: Vec<i64>,
    // Privacy context only: unrelated tabs in these windows are not policy targets.
    pub(in crate::host_impl) window_ids: Vec<i64>,
}

impl MutationScope {
    pub(in crate::host_impl) fn affected_tab_ids(&self, snapshot: &Value) -> Vec<i64> {
        let mut ids = self.tab_ids.clone();
        for id in &self.group_ids {
            ids.extend(affected_tab_ids(
                snapshot,
                "p:tab-group",
                &json!({"groupId":id}),
            ));
        }
        ids.sort_unstable();
        ids.dedup();
        ids
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Privacy {
    Regular,
    Private,
}

pub(super) fn classify(
    snapshot: &Value,
    action: &str,
    params: &Value,
    mut scope: MutationScope,
) -> Result<Option<Privacy>, String> {
    scope
        .tab_ids
        .extend(affected_tab_ids(snapshot, action, params));
    scope.window_ids.extend(params["windowId"].as_i64());
    scope
        .window_ids
        .extend(params["createProperties"]["windowId"].as_i64());
    let group_id = params["groupId"].as_i64();
    let mut private = false;
    let mut regular = false;
    let mut include = |incognito: bool| {
        private |= incognito;
        regular |= !incognito;
    };
    for tab in select_tabs_by_scope(
        snapshot,
        &json!({"tabIds":scope.affected_tab_ids(snapshot)}),
    )
    .tabs
    {
        include(tab.incognito);
    }
    for window in snapshot["windows"].as_array().into_iter().flatten() {
        let target_group = group_id.is_some_and(|id| {
            window["groups"].as_array().is_some_and(|groups| {
                groups
                    .iter()
                    .any(|group| group["groupId"].as_i64() == Some(id))
            })
        });
        if window["windowId"]
            .as_i64()
            .is_some_and(|id| scope.window_ids.contains(&id))
            || target_group
        {
            include(window["incognito"].as_bool() == Some(true));
        }
    }
    if action == "p:window-create" {
        include(params["incognito"].as_bool() == Some(true));
    }
    match (private, regular) {
        (true, true) => Err("Cannot mix private and regular browser scopes in one transaction; select a single privacy scope.".into()),
        (true, false) => Ok(Some(Privacy::Private)),
        (false, true) => Ok(Some(Privacy::Regular)),
        (false, false) => Ok(None),
    }
}
