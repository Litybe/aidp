use serde::{Deserialize, Serialize};

/// Counts of one project in the database.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoreStats {
    pub chunks: usize,
    pub files: usize,
    pub memories: usize,
    pub db_bytes: u64,
}
