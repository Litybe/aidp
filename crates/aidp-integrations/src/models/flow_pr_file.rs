//! The `pr.json` a flow writes once it opened the pull request of a ticket.

use serde::Serialize;

/// The pull request of a ticket branch, under the keys the flow facts read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FlowPrFile {
    pub pr_number: u64,
    pub pr_url: String,
    pub repository: String,
    pub branch: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub commit_sha: Option<String>,
}
