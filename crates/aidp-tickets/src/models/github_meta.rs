use crate::models::blocker_link::BlockerLink;
use crate::models::enums::issue_state::IssueState;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// GitHub-mirror metadata, present only when `source: github`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GithubMeta {
    /// Repository as `owner/name`.
    pub repo: String,
    pub number: u64,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub state: IssueState,
    #[serde(default)]
    pub synced_at: Option<DateTime<Utc>>,
    /// Open and closed blockers, from the body and from GitHub's native dependencies.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocked_by: Vec<BlockerLink>,
}
