use serde::{Deserialize, Serialize};

/// The team's repositories as last read from GitHub.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SquadReposCache {
    pub github_team: String,
    pub fetched_at_ms: i64,
    pub repos: Vec<String>,
}
