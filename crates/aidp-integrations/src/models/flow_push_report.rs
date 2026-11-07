//! What `flow git-push` prints.

use serde::Serialize;

/// The published branch, its remote head after the call, and whether the call created a commit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FlowPushReport {
    pub branch: String,
    pub sha: String,
    pub changed: bool,
}
