use crate::models::knowledge_proposal::KnowledgeProposal;
use serde::{Deserialize, Serialize};

/// The content of a `knowledge-proposals.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeProposalFile {
    pub proposals: Vec<KnowledgeProposal>,
}
