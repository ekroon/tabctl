//! GraphQL API for tabctl.
//!
//! Provides field-selective queries and mutations over tab data using
//! Juniper's synchronous executor. No async runtime or HTTP server needed.
//!
//! ## Public API
//!
//! - [`execute`] — run a GraphQL query/mutation against a snapshot
//! - [`schema_sdl`] — export the schema as SDL for agent discovery

mod context;
mod convert;
mod schema;
mod types;

pub use context::CommandSender;

use context::GqlContext;
use serde_json::Value;
use std::sync::Arc;

/// Execute a GraphQL query or mutation.
///
/// For queries, reads from the provided `snapshot` (a `p:snapshot` response).
/// For mutations, routes commands through `sender` and re-snapshots for
/// post-mutation result fields.
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

    let (result, errors) =
        juniper::execute_sync(query, None, &schema, &vars, &ctx).map_err(|e| e.to_string())?;

    let mut response = serde_json::Map::new();

    // Convert juniper::Value to serde_json::Value
    let data_json = serde_json::to_value(&result).unwrap_or(Value::Null);
    response.insert("data".to_string(), data_json);

    if !errors.is_empty() {
        let error_values: Vec<Value> = errors
            .iter()
            .map(|e| {
                serde_json::json!({
                    "message": e.error().message(),
                })
            })
            .collect();
        response.insert("errors".to_string(), Value::Array(error_values));
    }
    Ok(Value::Object(response))
}

/// Export the GraphQL schema as SDL (Schema Definition Language).
pub fn schema_sdl() -> String {
    let schema = schema::create_schema();
    schema.as_sdl()
}

/// Convert a serde_json::Value to a juniper InputValue.
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
