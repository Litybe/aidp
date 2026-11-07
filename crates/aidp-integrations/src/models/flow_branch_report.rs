//! What `flow git-branch` prints.

use serde::Serialize;

/// The working branch of a ticket and whether this call created it.
#[derive(Debug, Clone, Serialize)]
pub struct FlowBranchReport {
    pub branch: String,
    pub created: bool,
}
