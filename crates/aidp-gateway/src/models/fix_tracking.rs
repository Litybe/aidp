use std::collections::BTreeMap;
use std::path::PathBuf;

/// Everything the end-of-session handler needs once the request is gone.
pub struct FixTracking {
    pub repo: String,
    pub number: u64,
    pub session_id: String,
    pub threads: Vec<String>,
    pub worktree: PathBuf,
    pub owned_worktree: bool,
    pub project_path: PathBuf,
    /// Login of the PR author, whose replies count as answers.
    pub viewer: String,
    /// Comment count of each chosen thread when the session started.
    pub comment_counts: BTreeMap<String, usize>,
}
