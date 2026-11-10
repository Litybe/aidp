use crate::models::ingest_report::IngestRejection;
use serde::{Deserialize, Serialize};

/// Result of a synchronisation between the database and the vault notes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultSyncReport {
    /// Notes written from the database.
    pub exported: usize,
    /// Edited notes applied to the database.
    pub imported: usize,
    /// Notes that created a memory.
    pub created: usize,
    /// Memories archived because their note was deleted.
    pub archived: usize,
    /// Notes left untouched because they could not be applied.
    pub rejected: Vec<IngestRejection>,
}
