use serde::{Deserialize, Serialize};

/// What one Claude Code session asked for and changed, read from its transcript.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionJournal {
    pub session_id: String,
    pub branch: Option<String>,
    pub started: Option<String>,
    pub ended: Option<String>,
    pub prompts: Vec<String>,
    /// Every prompt of the session, including those past the kept ones.
    pub prompt_count: usize,
    pub files: Vec<String>,
    pub commits: Vec<String>,
    pub last_reply: Option<String>,
}
