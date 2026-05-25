use serde_json::Value;

mod analyze;
mod archive;
mod close;
mod focus;
mod group_assign;
mod group_gather;
mod group_ungroup;
mod group_update;
mod inspect;
mod list;
mod merge_window;
mod move_group;
mod move_tab;
mod open;
mod refresh;
pub(super) mod report;
pub(crate) mod resolve;
pub(crate) mod scope;
mod screenshot;
mod undo;

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
    /// Wait before continuing the orchestration.
    Delay { duration_ms: u64 },
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
    /// Emit a progress event without completing — the orchestration
    /// continues by immediately calling `step(Value::Null)`.
    Progress { data: Value },
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
        "focus" => match focus::FocusOrchestration::new(params) {
            Ok(o) => Some(Box::new(o)),
            Err(_) => Some(Box::new(ErrorOrchestration("Missing tabId".to_string()))),
        },
        "refresh" => match refresh::RefreshOrchestration::new(params) {
            Ok(o) => Some(Box::new(o)),
            Err(_) => Some(Box::new(ErrorOrchestration(
                "Missing tabId or tabIds".to_string(),
            ))),
        },
        "group-update" => Some(Box::new(group_update::GroupUpdateOrchestration::new(
            params,
        ))),
        "group-ungroup" => Some(Box::new(group_ungroup::GroupUngroupOrchestration::new(
            params,
        ))),
        "close" => Some(Box::new(close::CloseOrchestration::new(params))),
        "group-assign" => Some(Box::new(group_assign::GroupAssignOrchestration::new(
            params,
        ))),
        "group-gather" => Some(Box::new(group_gather::GroupGatherOrchestration::new(
            params,
        ))),
        "open" => Some(Box::new(open::OpenOrchestration::new(params))),
        "move-tab" => Some(Box::new(move_tab::MoveTabOrchestration::new(params))),
        "move-group" => Some(Box::new(move_group::MoveGroupOrchestration::new(params))),
        "archive" => Some(Box::new(archive::ArchiveOrchestration::new(params))),
        "merge-window" => Some(Box::new(merge_window::MergeWindowOrchestration::new(
            params,
        ))),
        "undo" => Some(Box::new(undo::UndoOrchestration::new(params))),
        "analyze" => Some(Box::new(analyze::AnalyzeOrchestration::new(params))),
        "inspect" => Some(Box::new(inspect::InspectOrchestration::new(params))),
        "report" => Some(Box::new(report::ReportOrchestration::new(params))),
        "screenshot" => Some(Box::new(screenshot::ScreenshotOrchestration::new(params))),
        "snapshot" => Some(Box::new(list::SnapshotOrchestration)),
        _ => None,
    }
}

/// Helper orchestration that immediately errors on start.
#[derive(Debug)]
struct ErrorOrchestration(String);

impl Orchestration for ErrorOrchestration {
    fn start(&mut self) -> OrchStep {
        OrchStep::Error {
            message: self.0.clone(),
            hint: None,
        }
    }
    fn step(&mut self, _response: Value) -> OrchStep {
        OrchStep::Error {
            message: self.0.clone(),
            hint: None,
        }
    }
}

/// Drive an orchestration to completion with a sequence of mock primitive
/// responses. Returns the final (response, undo) tuple. Panics on error or
/// if responses run out before completion.
#[cfg(test)]
fn drive_to_completion(
    orch: &mut dyn Orchestration,
    responses: &[serde_json::Value],
) -> (serde_json::Value, Option<serde_json::Value>) {
    let mut step = orch.start();
    let mut idx = 0;
    loop {
        match step {
            OrchStep::Complete { response, undo } => return (response, undo),
            OrchStep::Error { message, hint } => {
                panic!("orchestration error: {message} (hint: {hint:?})")
            }
            OrchStep::SendPrimitive { .. } => {
                assert!(
                    idx < responses.len(),
                    "ran out of mock responses at index {idx}"
                );
                step = orch.step(responses[idx].clone());
                idx += 1;
            }
            OrchStep::Progress { .. } => {
                step = orch.step(Value::Null);
            }
            OrchStep::Delay { .. } => {
                step = orch.step(Value::Null);
            }
        }
    }
}

#[cfg(test)]
mod graphql_contracts;
