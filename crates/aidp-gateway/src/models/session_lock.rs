use crate::flows::lock::WorkspaceLock;
use crate::sessions::SessionManager;
use std::sync::Arc;

/// The lock backed by the session manager's admission, so both sides exclude each other.
pub struct SessionLock {
    sessions: Arc<SessionManager>,
}

impl SessionLock {
    pub fn new(sessions: Arc<SessionManager>) -> Self {
        SessionLock { sessions }
    }
}

impl WorkspaceLock for SessionLock {
    fn try_acquire(&self, workspace_key: &str) -> bool {
        self.sessions.try_hold_workspace(workspace_key)
    }

    fn try_acquire_for_stage(&self, workspace_key: &str, stage: &str) -> bool {
        self.sessions
            .try_hold_workspace_for_stage(workspace_key, stage)
    }

    fn release(&self, workspace_key: &str) {
        self.sessions.release_workspace(workspace_key);
    }

    fn is_busy(&self, workspace_key: &str) -> bool {
        self.sessions.is_workspace_busy(workspace_key)
    }
}

#[cfg(test)]
#[path = "../../tests/unit/session_lock_tests.rs"]
mod tests;
