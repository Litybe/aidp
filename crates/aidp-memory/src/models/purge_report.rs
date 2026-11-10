use serde::{Deserialize, Serialize};

/// Result of `rag purge`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PurgeReport {
    pub chunks: usize,
    pub files: usize,
    /// False when the database was busy and kept its free pages for later writes.
    pub compacted: bool,
}
