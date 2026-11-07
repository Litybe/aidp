pub use crate::models::enums::{FlowError, FlowStatus, FrameKind, OnError, StepKind, StepStatus};
pub use crate::models::{FlowDef, FlowState, Frame, Step, StepState};

/// Ticket source value that `skip_on: local` matches.
pub const LOCAL_SOURCE: &str = "local";
/// Name of the block inserted after a step that pushes.
pub const CI_BLOCK: &str = "ci";
/// Prefix of the id of an inserted CI block reference.
pub const CI_STEP_PREFIX: &str = "ci@";
/// Name of the block inserted after a step that pushes, before the `ci` block: the host publishes.
pub const PUBLISH_BLOCK: &str = "publish";
/// Prefix of the id of an inserted publish block reference.
pub const PUBLISH_STEP_PREFIX: &str = "publish@";

#[cfg(test)]
#[path = "../tests/unit/model_tests.rs"]
mod tests;
