//! Result of a merge attempt on a pull request.

use serde::Serialize;

/// `rebased` means the branch was behind and was updated instead of merged.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MergeOutcome {
    pub merged: bool,
    pub rebased: bool,
    pub sha: Option<String>,
    pub branch_deleted: bool,
    pub detail: Vec<String>,
}
