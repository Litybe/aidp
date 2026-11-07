use aidp_integrations::models::squad_comment::SquadComment;
use serde::{Deserialize, Serialize};

/// Everything the caller knows about one pull request, ready for signal computation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttentionFacts {
    pub repo: String,
    pub number: u64,
    pub head_sha: String,
    pub me: String,
    pub author: String,
    pub last_seen: Option<String>,
    pub comments: Vec<SquadComment>,
    pub bot_authors: Vec<String>,
    pub changes_requested_by: Vec<String>,
    pub review_requests: Vec<String>,
    pub conflict: bool,
    pub gate_ok: bool,
}
