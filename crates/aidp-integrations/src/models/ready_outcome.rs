//! Result of marking a pull request ready for review or back to draft.

use serde::Serialize;

/// Whether the call mutated the PR, and the draft state it ends in.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReadyOutcome {
    pub changed: bool,
    pub draft: bool,
}
