use serde_json::Value;
use std::sync::Arc;

/// Trait for sending commands to the host during mutations.
///
/// Implementors provide the transport (socket, mock, etc.).
/// The GraphQL crate never touches sockets directly.
pub trait CommandSender: Send + Sync {
    /// Send a command and return the response data.
    fn send(&self, action: &str, params: Value) -> Result<Value, String>;

    /// Take a fresh snapshot (used after mutations for result fields).
    fn snapshot(&self) -> Result<Value, String> {
        self.send("list", Value::Object(serde_json::Map::new()))
    }
}

/// Juniper execution context.
pub(crate) struct GqlContext {
    pub(crate) snapshot: Value,
    pub(crate) sender: Arc<dyn CommandSender>,
}

impl GqlContext {
    pub(crate) fn new(snapshot: Value, sender: Arc<dyn CommandSender>) -> Self {
        Self { snapshot, sender }
    }
}

impl juniper::Context for GqlContext {}
