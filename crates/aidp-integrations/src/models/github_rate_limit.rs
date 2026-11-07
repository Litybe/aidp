use crate::models::github_rate_bucket::GithubRateBucket;
use serde::{Deserialize, Serialize};

/// The GitHub quotas of the authenticated user; a quota GitHub did not report is absent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GithubRateLimit {
    pub graphql: Option<GithubRateBucket>,
    pub core: Option<GithubRateBucket>,
    pub fetched_at: String,
}
