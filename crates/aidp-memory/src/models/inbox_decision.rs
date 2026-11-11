use crate::models::enums::ProposalOutcome;
use serde::{Deserialize, Serialize};

/// What approving or rejecting an inbox note did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboxDecision {
    pub outcome: Option<ProposalOutcome>,
    /// Vault-relative path of the note after the move.
    pub path: String,
}
