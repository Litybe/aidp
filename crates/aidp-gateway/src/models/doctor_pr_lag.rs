use serde::{Deserialize, Serialize};
use serde_json::Value;

/// How far a branch lags behind `main` and whether it still merges with it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DoctorPrLag {
    pub branch: String,
    /// Commits of `origin/main` the branch does not have.
    pub behind_by: Option<u64>,
    /// True when a merge of `origin/main` into the branch is clean; `None` when git could not say.
    pub mergeable: Option<bool>,
    /// Files that conflict in that merge.
    pub conflicts: Vec<String>,
    /// The `mergeable` field of the live PR status, as GitHub gave it.
    pub pr_mergeable: Option<Value>,
    /// Why nothing could be computed.
    pub unavailable: Option<String>,
}
