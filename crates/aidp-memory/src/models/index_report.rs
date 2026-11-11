use serde::{Deserialize, Serialize};

/// Result of one indexing pass.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexReport {
    /// Files seen for the first time.
    pub indexed: usize,
    /// Files whose content changed and were re-chunked.
    pub updated: usize,
    /// Files that disappeared and lost their chunks.
    pub removed: usize,
    /// Files skipped because mtime or hash did not change.
    pub unchanged: usize,
    /// Chunks of the project after the pass.
    pub chunks: usize,
    /// Chunks embedded during the pass.
    pub embedded: usize,
    pub duration_ms: u64,
    /// Requested sources that have no directory to read.
    pub skipped_sources: Vec<String>,
}
