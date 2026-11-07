//! One writer per workspace; the lock is shared with session admission.

use crate::worktrees::{worktree_path, Workspace, WorkspaceKind};
use std::path::Path;

/// Admission key of the worktree of ticket `key`, the same key sessions use.
pub fn workspace_key(vault: &Path, repo: &str, key: &str) -> String {
    Workspace {
        kind: WorkspaceKind::Ticket,
        path: worktree_path(vault, repo, key),
        branch: None,
        name: None,
    }
    .key()
}

/// A lock on workspace keys.
pub trait WorkspaceLock: Send + Sync {
    /// Takes the workspace; false when someone else holds it or a session runs there.
    fn try_acquire(&self, workspace_key: &str) -> bool;

    /// Takes the workspace on behalf of `stage`; locks that track holders record it.
    /// The default ignores the stage and behaves like [`Self::try_acquire`].
    fn try_acquire_for_stage(&self, workspace_key: &str, _stage: &str) -> bool {
        self.try_acquire(workspace_key)
    }

    fn release(&self, workspace_key: &str);

    fn is_busy(&self, workspace_key: &str) -> bool;
}
pub use crate::models::{LocalLock, SessionLock, WorkspaceGuard};
