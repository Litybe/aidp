use serde::{Deserialize, Serialize};

/// One file of a bug fix and the lines it added and deleted (`git diff --numstat`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoctorFileChange {
    pub path: String,
    /// Lines added; `None` for a binary file.
    pub added: Option<u64>,
    /// Lines deleted; `None` for a binary file.
    pub deleted: Option<u64>,
}
