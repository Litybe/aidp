//! The input of the GitHub `createCommitOnBranch` mutation.

use crate::models::commit_message::CommitMessage;
use crate::models::committable_branch::CommittableBranch;
use crate::models::file_changes::FileChanges;
use serde::Serialize;

/// One commit on `branch`, accepted only while the branch still points at `expected_head_oid`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateCommitOnBranchInput {
    pub branch: CommittableBranch,
    pub message: CommitMessage,
    pub expected_head_oid: String,
    pub file_changes: FileChanges,
}
