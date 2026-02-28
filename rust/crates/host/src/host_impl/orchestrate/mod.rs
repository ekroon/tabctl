use serde_json::Value;

mod list;

/// Multi-step orchestration of extension primitives for a single CLI request.
///
/// Each implementation encodes a state machine: `start()` produces the first
/// primitive call, and `step(response)` advances the machine until it reaches
/// `Complete` or `Error`.
pub(super) trait Orchestration: Send + std::fmt::Debug {
    /// Produce the first primitive action to send to the extension.
    fn start(&mut self) -> OrchStep;

    /// Given the extension's response to the previous primitive, produce the
    /// next step. Called repeatedly until `Complete` or `Error` is returned.
    fn step(&mut self, response: Value) -> OrchStep;
}

/// A single step in an orchestration sequence.
#[derive(Debug)]
#[allow(dead_code)] // Variants used as commands migrate to orchestration
pub(super) enum OrchStep {
    /// Send a `p:`-prefixed primitive action to the extension.
    SendPrimitive { action: String, params: Value },
    /// Orchestration succeeded — respond to CLI client.
    Complete {
        response: Value,
        undo: Option<Value>,
    },
    /// Orchestration failed — respond with error.
    Error {
        message: String,
        hint: Option<String>,
    },
}

/// Factory: returns an Orchestration impl for the given CLI action, or `None`
/// to fall through to legacy extension forwarding.
///
/// Commands are registered here as they migrate from thick extension handlers
/// to host-side orchestration.
pub(super) fn orchestration_for(action: &str, params: &Value) -> Option<Box<dyn Orchestration>> {
    match action {
        "list" => Some(Box::new(list::ListOrchestration::new(params))),
        "group-list" => Some(Box::new(list::GroupListOrchestration::new(params))),
        _ => None,
    }
}
