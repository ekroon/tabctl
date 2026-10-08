use crate::context::{CommandSender, GqlContext};
use crate::schema;
use serde_json::Value;
use std::sync::Arc;

/// Execute a query or mutation while retaining GraphQL's partial-data and error envelope.
pub fn execute(
    query: &str,
    variables: Option<&str>,
    snapshot: Value,
    sender: Arc<dyn CommandSender>,
) -> Result<Value, String> {
    let ctx = GqlContext::new(snapshot, sender);
    let schema = schema::create_schema();
    let vars = match variables {
        Some(v) if !v.is_empty() => {
            let parsed: Value =
                serde_json::from_str(v).map_err(|e| format!("Invalid variables JSON: {e}"))?;
            let obj = parsed
                .as_object()
                .ok_or("Variables must be a JSON object")?;
            let mut map = juniper::Variables::new();
            for (key, val) in obj {
                map.insert(key.clone(), json_to_input_value(val));
            }
            map
        }
        _ => juniper::Variables::new(),
    };

    let (result, errors) = match juniper::execute_sync(query, None, &schema, &vars, &ctx) {
        Ok(result) => result,
        Err(error) => {
            return Ok(serde_json::json!({
                "data": null,
                "errors": [{ "message": error.to_string() }]
            }))
        }
    };
    let mut response = serde_json::Map::new();
    response.insert(
        "data".to_string(),
        serde_json::to_value(result)
            .map_err(|e| format!("GraphQL result serialization failed: {e}"))?,
    );
    if !errors.is_empty() {
        response.insert(
            "errors".to_string(),
            serde_json::to_value(errors)
                .map_err(|e| format!("GraphQL error serialization failed: {e}"))?,
        );
    }
    Ok(Value::Object(response))
}

fn json_to_input_value(val: &Value) -> juniper::InputValue {
    match val {
        Value::Null => juniper::InputValue::null(),
        Value::Bool(b) => juniper::InputValue::scalar(*b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                juniper::InputValue::scalar(i as i32)
            } else {
                juniper::InputValue::scalar(n.as_f64().unwrap_or(0.0))
            }
        }
        Value::String(s) => juniper::InputValue::scalar(s.clone()),
        Value::Array(arr) => {
            juniper::InputValue::list(arr.iter().map(json_to_input_value).collect())
        }
        Value::Object(obj) => {
            let entries: indexmap::IndexMap<&str, juniper::InputValue> = obj
                .iter()
                .map(|(k, v)| (k.as_str(), json_to_input_value(v)))
                .collect();
            juniper::InputValue::object(entries)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct OfflineSender;

    impl CommandSender for OfflineSender {
        fn send(&self, _action: &str, _params: Value) -> Result<Value, String> {
            Err("Browser channel unavailable".to_string())
        }
    }

    #[test]
    fn resolver_errors_keep_partial_data_and_error_paths() {
        let response = execute(
            "{ windows { windowId } latestBrowserState { snapshotId } }",
            None,
            serde_json::json!({ "windows": [{ "windowId": 4, "tabs": [] }] }),
            Arc::new(OfflineSender),
        )
        .unwrap();
        assert_eq!(response["data"]["windows"][0]["windowId"], 4);
        assert!(response["data"]["latestBrowserState"].is_null());
        assert_eq!(
            response["errors"][0]["message"],
            "Browser channel unavailable"
        );
        assert_eq!(response["errors"][0]["path"][0], "latestBrowserState");
    }

    #[test]
    fn invalid_graphql_keeps_the_error_envelope() {
        for query in ["{ missingField }", "{"] {
            let response =
                execute(query, None, serde_json::json!({}), Arc::new(OfflineSender)).unwrap();
            assert!(response["data"].is_null());
            assert!(!response["errors"][0]["message"]
                .as_str()
                .unwrap()
                .is_empty());
        }
    }

    #[test]
    fn private_close_projects_success_and_the_reason_undo_is_unavailable() {
        struct PrivateCloseSender;
        impl CommandSender for PrivateCloseSender {
            fn send(&self, action: &str, params: Value) -> Result<Value, String> {
                match action {
                    "close" => {
                        assert_eq!(
                            params,
                            serde_json::json!({"tabIds": [7], "confirmed": true})
                        );
                        Ok(serde_json::json!({
                            "txid": null,
                            "undoUnavailable": "private tabs are never persisted",
                            "summary": {"closedTabs": 1}
                        }))
                    }
                    "list" => Ok(serde_json::json!({"windows": []})),
                    _ => Err(format!("Unexpected contract-test action: {action}")),
                }
            }
        }

        let response = execute(
            "mutation { closeTabs(tabIds: [7], confirm: true) {
                closedTabs txid undoUnavailable remainingTabs { tabId }
            } }",
            None,
            serde_json::json!({}),
            Arc::new(PrivateCloseSender),
        )
        .unwrap();
        assert_eq!(
            response,
            serde_json::json!({"data": {"closeTabs": {
                "closedTabs": 1,
                "txid": null,
                "undoUnavailable": "private tabs are never persisted",
                "remainingTabs": []
            }}})
        );
    }
}
