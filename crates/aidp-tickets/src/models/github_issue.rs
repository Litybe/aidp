use crate::models::enums::issue_state::IssueState;
use crate::models::milestone::Milestone;
use chrono::{DateTime, Utc};

/// The projection of a GitHub issue that the sync and the routing read.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GithubIssue {
    pub number: u64,
    pub title: String,
    pub body: String,
    pub labels: Vec<String>,
    pub milestone: Option<Milestone>,
    pub state: IssueState,
    pub url: String,
    pub created_at: Option<DateTime<Utc>>,
    pub updated_at: Option<DateTime<Utc>>,
}

/// One issue a GitHub issue depends on, from its native `blocked_by` dependencies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueDependency {
    /// Repository as `owner/name`.
    pub repo: String,
    pub number: u64,
    pub state: IssueState,
}
