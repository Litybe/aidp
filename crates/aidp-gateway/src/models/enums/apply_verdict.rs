/// What applying an isolated task's changes back to the ticket worktree did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyVerdict {
    /// The changes landed, or there was nothing to apply.
    Applied,
    /// The changes do not fit the ticket worktree; it was left untouched. Holds the paths in conflict.
    Conflict(Vec<String>),
}
