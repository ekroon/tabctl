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
    _variables: Option<&str>,
    snapshot: Value,
    sender: Arc<dyn CommandSender>,
) -> Result<Value, String> {
    let ctx = GqlContext::new(snapshot, sender);
    let schema = schema::create_schema();
    let vars = juniper::Variables::new();

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
