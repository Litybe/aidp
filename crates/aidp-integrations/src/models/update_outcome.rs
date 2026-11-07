//! Result of rebasing a pull request branch onto its base.

use serde::Serialize;

/// The head commit of the PR after the rebase.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateOutcome {
    pub head_sha: Option<String>,
}
