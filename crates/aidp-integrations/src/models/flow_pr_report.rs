//! What `flow pr-create` prints.

use serde::Serialize;

/// The `pr.json` path a flow reads its pull request from, the pull request, and whether this call opened it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FlowPrReport {
    pub pr: String,
    pub number: u64,
    pub url: String,
    pub branch: String,
    pub changed: bool,
}
