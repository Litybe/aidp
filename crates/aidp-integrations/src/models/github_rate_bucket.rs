use serde::{Deserialize, Serialize};

/// One GitHub API quota (GraphQL or REST) as `gh api rate_limit` reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubRateBucket {
    pub limit: u64,
    pub remaining: u64,
    pub used: u64,
    /// When the quota is restored, RFC 3339.
    pub reset_at: String,
}
