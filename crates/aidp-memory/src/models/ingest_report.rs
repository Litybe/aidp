use serde::{Deserialize, Serialize};

/// A proposals file that was refused, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IngestRejection {
    pub file: String,
    pub reason: String,
}

/// Result of `memory ingest`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestReport {
    pub files: usize,
    pub created: usize,
    pub deduplicated: usize,
    pub activated: usize,
    pub rejected: Vec<IngestRejection>,
}
