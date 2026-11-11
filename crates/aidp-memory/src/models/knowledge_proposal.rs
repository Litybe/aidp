use crate::models::constants::knowledge_proposals::DEFAULT_PROPOSAL_IMPORTANCE;
use crate::models::enums::{Domain, ProposalAction, ProposalScope};
use serde::{Deserialize, Serialize};

fn default_importance() -> f64 {
    DEFAULT_PROPOSAL_IMPORTANCE
}

/// One note a stage proposes to add, rewrite or replace.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnowledgeProposal {
    pub action: ProposalAction,
    pub domain: Domain,
    pub scope: ProposalScope,
    pub slug: String,
    pub title: String,
    pub body: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub agents: Vec<String>,
    #[serde(default = "default_importance")]
    pub importance: f64,
    /// Note path relative to `Knowledge/`, required by `update` and `supersede`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
}
