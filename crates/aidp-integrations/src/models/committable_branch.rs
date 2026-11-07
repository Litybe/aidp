//! The branch a `createCommitOnBranch` mutation commits to.

use serde::Serialize;

/// A branch named by its repository `owner/name` and its short name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommittableBranch {
    pub repository_name_with_owner: String,
    pub branch_name: String,
}
