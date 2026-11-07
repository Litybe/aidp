use crate::flows::lock::WorkspaceLock;
use crate::sync::lock;
use std::collections::HashSet;
use std::sync::Mutex;

/// A process-local lock, for a daemon without sessions and for tests.
#[derive(Default)]
pub struct LocalLock {
    held: Mutex<HashSet<String>>,
}

impl WorkspaceLock for LocalLock {
    fn try_acquire(&self, workspace_key: &str) -> bool {
        lock(&self.held).insert(workspace_key.to_string())
    }

    fn release(&self, workspace_key: &str) {
        lock(&self.held).remove(workspace_key);
    }

    fn is_busy(&self, workspace_key: &str) -> bool {
        lock(&self.held).contains(workspace_key)
    }
}
