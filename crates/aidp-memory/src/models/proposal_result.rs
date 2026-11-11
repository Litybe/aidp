use crate::models::enums::ProposalOutcome;
use serde::{Deserialize, Serialize};

/// What ingestion did with one proposal and where the result lives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposalResult {
    pub slug: String,
    pub outcome: ProposalOutcome,
    /// Vault-relative path of the note written or changed.
    pub path: String,
}
