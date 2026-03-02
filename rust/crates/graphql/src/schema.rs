use juniper::{graphql_object, EmptySubscription, FieldResult, RootNode};

use crate::context::GqlContext;
use crate::convert::{tab_from_value, windows_from_snapshot};
use crate::types::*;

pub(crate) type Schema = RootNode<'static, Query, Mutation, EmptySubscription<GqlContext>>;

pub(crate) fn create_schema() -> Schema {
    Schema::new(Query, Mutation, EmptySubscription::new())
}

/// Query root — read-only access to tab data from a snapshot.
const DEFAULT_LIMIT: i32 = 20;

pub(crate) struct Query;

#[graphql_object(context = GqlContext)]
impl Query {
    /// All windows with their tabs and groups.
    fn windows(ctx: &GqlContext) -> Vec<Window> {
        windows_from_snapshot(&ctx.snapshot)
    }

    /// A single window by ID.
    fn window(ctx: &GqlContext, id: i32) -> Option<Window> {
        windows_from_snapshot(&ctx.snapshot)
            .into_iter()
            .find(|w| w.window_id == id)
    }

    /// Paginated tabs with optional scope filters.
    fn tabs(
        ctx: &GqlContext,
        window_id: Option<i32>,
        group_id: Option<i32>,
        group_title: Option<String>,
        ungrouped: Option<bool>,
        limit: Option<i32>,
        offset: Option<i32>,
    ) -> TabPage {
        let windows = windows_from_snapshot(&ctx.snapshot);
        let mut tabs: Vec<Tab> = windows.into_iter().flat_map(|w| w.tabs).collect();

        if let Some(wid) = window_id {
            tabs.retain(|t| t.window_id == wid);
        }
        if let Some(gid) = group_id {
            tabs.retain(|t| t.group_id == gid);
        }
        if let Some(ref title) = group_title {
            tabs.retain(|t| t.group_title.as_deref() == Some(title.as_str()));
        }
        if ungrouped == Some(true) {
            tabs.retain(|t| t.group_id == -1);
        }

        paginate_tabs(tabs, limit, offset)
    }

    /// A single tab by ID.
    fn tab(ctx: &GqlContext, id: i32) -> Option<Tab> {
        windows_from_snapshot(&ctx.snapshot)
            .into_iter()
            .flat_map(|w| w.tabs)
            .find(|t| t.tab_id == id)
    }

    /// Groups filtered by optional window ID.
    fn groups(ctx: &GqlContext, window_id: Option<i32>) -> Vec<Group> {
        let windows = windows_from_snapshot(&ctx.snapshot);
        windows
            .into_iter()
            .filter(|w| window_id.is_none() || Some(w.window_id) == window_id)
            .flat_map(|w| w.groups)
            .collect()
    }
}

fn paginate_tabs(tabs: Vec<Tab>, limit: Option<i32>, offset: Option<i32>) -> TabPage {
    let total = tabs.len() as i32;
    let off = offset.unwrap_or(0).max(0) as usize;
    let lim = limit.unwrap_or(DEFAULT_LIMIT).max(0) as usize;

    let page: Vec<Tab> = tabs.into_iter().skip(off).take(lim).collect();
    let has_more = (off + page.len()) < total as usize;

    TabPage {
        items: page,
        total,
        offset: off as i32,
        has_more,
    }
}

/// Mutation root — tab actions with result projection.
pub(crate) struct Mutation;

#[graphql_object(context = GqlContext)]
impl Mutation {
    /// Close tabs by ID. Returns the transaction ID and remaining tabs.
    fn close_tabs(
        ctx: &GqlContext,
        tab_ids: Vec<i32>,
        confirm: Option<bool>,
        dry_run: Option<bool>,
    ) -> FieldResult<CloseResult> {
        let mut params = serde_json::Map::new();
        params.insert(
            "tabIds".to_string(),
            serde_json::json!(tab_ids.iter().map(|&id| id as i64).collect::<Vec<_>>()),
        );
        if confirm == Some(true) {
            params.insert("confirmed".to_string(), serde_json::json!(true));
        }
        if dry_run == Some(true) {
            params.insert("dryRun".to_string(), serde_json::json!(true));
        }

        let response = ctx
            .sender
            .send("close", serde_json::Value::Object(params))
            .map_err(|e| juniper::FieldError::new(e, juniper::Value::Null))?;

        let txid = response
            .get("txid")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let closed_tabs = response
            .get("summary")
            .and_then(|s| s.get("closedTabs"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32;

        // Re-snapshot for remaining tabs
        let remaining_tabs = match ctx.sender.snapshot() {
            Ok(snap) => windows_from_snapshot(&snap)
                .into_iter()
                .flat_map(|w| w.tabs)
                .collect(),
            Err(e) => {
                return Err(juniper::FieldError::new(
                    format!("Tabs closed (txid: {txid}) but post-mutation snapshot failed: {e}"),
                    juniper::Value::Null,
                ));
            }
        };

        Ok(CloseResult {
            txid,
            closed_tabs,
            remaining_tabs,
        })
    }

    /// Open new tabs. Returns the created tabs.
    fn open_tabs(
        ctx: &GqlContext,
        urls: Vec<String>,
        window_id: Option<i32>,
        group: Option<String>,
    ) -> FieldResult<OpenResult> {
        let mut params = serde_json::Map::new();
        params.insert("urls".to_string(), serde_json::json!(urls));
        if let Some(wid) = window_id {
            params.insert("windowId".to_string(), serde_json::json!(wid as i64));
        }
        if let Some(ref g) = group {
            params.insert("groupTitle".to_string(), serde_json::json!(g));
        }

        let response = ctx
            .sender
            .send("open", serde_json::Value::Object(params))
            .map_err(|e| juniper::FieldError::new(e, juniper::Value::Null))?;

        let tabs = response
            .get("tabs")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|t| {
                        let wid = t.get("windowId").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
                        tab_from_value(t, wid)
                    })
                    .collect()
            })
            .unwrap_or_default();

        Ok(OpenResult { tabs })
    }

    /// Refresh (reload) tabs by ID.
    fn refresh_tabs(ctx: &GqlContext, tab_ids: Vec<i32>) -> FieldResult<RefreshResult> {
        let mut params = serde_json::Map::new();
        params.insert(
            "tabIds".to_string(),
            serde_json::json!(tab_ids.iter().map(|&id| id as i64).collect::<Vec<_>>()),
        );

        let response = ctx
            .sender
            .send("refresh", serde_json::Value::Object(params))
            .map_err(|e| juniper::FieldError::new(e, juniper::Value::Null))?;

        let refreshed_tabs = response
            .get("summary")
            .and_then(|s| s.get("refreshedTabs"))
            .and_then(|v| v.as_i64())
            .unwrap_or(0) as i32;

        Ok(RefreshResult { refreshed_tabs })
    }
}

#[cfg(test)]
mod tests {
    use crate::context::CommandSender;
    use std::sync::Arc;

    struct MockSender;

    impl CommandSender for MockSender {
        fn send(
            &self,
            _action: &str,
            _params: serde_json::Value,
        ) -> Result<serde_json::Value, String> {
            Ok(serde_json::json!({
                "txid": "tx-test-1",
                "summary": { "closedTabs": 2 }
            }))
        }

        fn snapshot(&self) -> Result<serde_json::Value, String> {
            Ok(sample_snapshot())
        }
    }

    fn sample_snapshot() -> serde_json::Value {
        serde_json::json!({
            "windows": [{
                "windowId": 100,
                "focused": true,
                "tabs": [
                    {"tabId": 1, "windowId": 100, "index": 0, "url": "https://a.com", "title": "A", "active": true, "pinned": false, "groupId": 10, "groupTitle": "Work"},
                    {"tabId": 2, "windowId": 100, "index": 1, "url": "https://b.com", "title": "B", "active": false, "pinned": true, "groupId": -1},
                    {"tabId": 3, "windowId": 100, "index": 2, "url": "https://c.com", "title": "C", "active": false, "pinned": false, "groupId": 10, "groupTitle": "Work"}
                ],
                "groups": [{"groupId": 10, "title": "Work", "color": "blue", "collapsed": false}]
            }, {
                "windowId": 200,
                "focused": false,
                "tabs": [
                    {"tabId": 4, "windowId": 200, "index": 0, "url": "https://d.com", "title": "D", "active": true, "pinned": false, "groupId": -1}
                ],
                "groups": []
            }]
        })
    }

    fn exec(query: &str) -> serde_json::Value {
        crate::execute(query, None, sample_snapshot(), Arc::new(MockSender)).unwrap()
    }

    #[test]
    fn query_windows_returns_all() {
        let result = exec("{ windows { windowId focused tabCount } }");
        let windows = result["data"]["windows"].as_array().unwrap();
        assert_eq!(windows.len(), 2);
    }

    #[test]
    fn query_window_by_id() {
        let result = exec("{ window(id: 100) { windowId tabs { tabId url } } }");
        assert!(!result["data"]["window"].is_null());
        let tabs = result["data"]["window"]["tabs"].as_array().unwrap();
        assert_eq!(tabs.len(), 3);
    }

    #[test]
    fn query_tabs_with_group_title_filter() {
        let result = exec(r#"{ tabs(groupTitle: "Work") { items { tabId } total hasMore } }"#);
        let tabs = result["data"]["tabs"]["items"].as_array().unwrap();
        assert_eq!(tabs.len(), 2);
        assert_eq!(result["data"]["tabs"]["total"], 2);
        assert_eq!(result["data"]["tabs"]["hasMore"], false);
    }

    #[test]
    fn query_tabs_ungrouped() {
        let result = exec("{ tabs(ungrouped: true) { items { tabId } total } }");
        let tabs = result["data"]["tabs"]["items"].as_array().unwrap();
        assert_eq!(tabs.len(), 2);
    }

    #[test]
    fn query_tab_by_id() {
        let result = exec("{ tab(id: 2) { tabId url title } }");
        assert!(!result["data"]["tab"].is_null());
        assert_eq!(result["data"]["tab"]["url"], "https://b.com");
    }

    #[test]
    fn query_groups_filtered_by_window() {
        let result = exec("{ groups(windowId: 100) { groupId title tabCount } }");
        let groups = result["data"]["groups"].as_array().unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0]["tabCount"], 2);
    }

    #[test]
    fn introspection_returns_tab_type() {
        let result = exec(r#"{ __type(name: "Tab") { name fields { name } } }"#);
        assert!(!result["data"]["__type"].is_null());
        let fields = result["data"]["__type"]["fields"].as_array().unwrap();
        let field_names: Vec<&str> = fields.iter().filter_map(|f| f["name"].as_str()).collect();
        assert!(field_names.contains(&"tabId"));
        assert!(field_names.contains(&"url"));
        assert!(field_names.contains(&"title"));
    }

    #[test]
    fn tabs_default_limit_and_pagination() {
        let result = exec("{ tabs { items { tabId } total offset hasMore } }");
        let page = &result["data"]["tabs"];
        let items = page["items"].as_array().unwrap();
        // Sample snapshot has 4 tabs total, all fit in default limit 20
        assert_eq!(items.len(), 4);
        assert_eq!(page["total"], 4);
        assert_eq!(page["offset"], 0);
        assert_eq!(page["hasMore"], false);
    }

    #[test]
    fn tabs_limit_and_offset() {
        let result = exec("{ tabs(limit: 2, offset: 1) { items { tabId } total offset hasMore } }");
        let page = &result["data"]["tabs"];
        let items = page["items"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(page["total"], 4);
        assert_eq!(page["offset"], 1);
        assert_eq!(page["hasMore"], true);
    }

    #[test]
    fn mutation_close_tabs() {
        let result =
            exec("mutation { closeTabs(tabIds: [1, 2], confirm: true) { txid closedTabs } }");
        assert!(result.get("errors").is_none());
        assert_eq!(result["data"]["closeTabs"]["txid"], "tx-test-1");
        assert_eq!(result["data"]["closeTabs"]["closedTabs"], 2);
    }

    #[test]
    fn schema_sdl_contains_types() {
        let sdl = crate::schema_sdl();
        assert!(sdl.contains("type Tab"));
        assert!(sdl.contains("type Window"));
        assert!(sdl.contains("type Group"));
        assert!(sdl.contains("type Query"));
        assert!(sdl.contains("type Mutation"));
    }
}
