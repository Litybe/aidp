use crate::models::ingest_report::IngestRejection;
use crate::models::proposal_result::ProposalResult;
use serde::{Deserialize, Serialize};

/// Result of ingesting `knowledge-proposals.json` files.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeIngestReport {
    pub files: usize,
    pub added: usize,
    pub merged: usize,
    pub inboxed: usize,
    pub updated: usize,
    pub superseded: usize,
    pub results: Vec<ProposalResult>,
    pub rejected: Vec<IngestRejection>,
}

impl KnowledgeIngestReport {
    /// Notes written or changed in the vault.
    pub fn changed(&self) -> usize {
        self.added + self.merged + self.inboxed + self.updated + self.superseded
    }

    /// Adds the counts and lists of another report.
    pub fn absorb(&mut self, other: KnowledgeIngestReport) {
        self.files += other.files;
        self.added += other.added;
        self.merged += other.merged;
        self.inboxed += other.inboxed;
        self.updated += other.updated;
        self.superseded += other.superseded;
        self.results.extend(other.results);
        self.rejected.extend(other.rejected);
    }
}
