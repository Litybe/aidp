use serde::{Deserialize, Serialize};

/// A test-infrastructure file a branch changed against `origin/main`, with its sizes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoctorInfraFile {
    pub path: String,
    /// `M`odified, `A`dded or `D`eleted by the branch.
    pub status: String,
    /// Bytes at the merge base with `origin/main`; `None` for an added file.
    pub before_bytes: Option<u64>,
    /// Bytes on the branch; `None` for a deleted file.
    pub after_bytes: Option<u64>,
    /// Deleted, or shrunk by 30 % or more: the signature of a re-saved scene that lost objects.
    pub suspect: bool,
}
