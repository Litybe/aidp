use std::path::PathBuf;

/// An isolated worktree prepared for one plan task: its directory and the commit it starts from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskWorktree {
    pub path: PathBuf,
    /// Commit the task started from: the ticket HEAD, plus the carried dirty state when any.
    pub base: String,
}
