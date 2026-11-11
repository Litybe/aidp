//! The port the compiler asks for the knowledge block of a stage.

use crate::models::stage_context_request::StageContextRequest;

/// Builds the knowledge notes a stage prompt carries. The implementation lives in `aidp-memory`.
pub trait ContextProvider {
    /// The block body for the stage, without heading; `None` when nothing is relevant or on error.
    fn stage_context(&self, request: &StageContextRequest) -> Option<String>;
}
