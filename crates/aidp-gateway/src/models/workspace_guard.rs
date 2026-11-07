use crate::flows::lock::WorkspaceLock;
use std::sync::Arc;

/// Work run once when the guard releases its workspace.
type ReleaseHook = Box<dyn FnOnce() + Send + Sync>;

/// Releases its workspace when dropped.
#[must_use = "the workspace is released as soon as the guard is dropped"]
pub struct WorkspaceGuard {
    lock: Arc<dyn WorkspaceLock>,
    workspace_key: String,
    release_hook: Option<ReleaseHook>,
}

impl WorkspaceGuard {
    /// A guard on `workspace_key`, or `None` when the workspace is busy.
    pub fn acquire(lock: &Arc<dyn WorkspaceLock>, workspace_key: &str) -> Option<WorkspaceGuard> {
        lock.try_acquire(workspace_key).then(|| WorkspaceGuard {
            lock: lock.clone(),
            workspace_key: workspace_key.to_string(),
            release_hook: None,
        })
    }

    /// A guard on `workspace_key` held on behalf of `stage`, or `None` when the workspace is busy.
    pub fn acquire_for_stage(
        lock: &Arc<dyn WorkspaceLock>,
        workspace_key: &str,
        stage: &str,
    ) -> Option<WorkspaceGuard> {
        lock.try_acquire_for_stage(workspace_key, stage)
            .then(|| WorkspaceGuard {
                lock: lock.clone(),
                workspace_key: workspace_key.to_string(),
                release_hook: None,
            })
    }

    /// Runs `hook` when the guard releases the workspace, before the lock is freed.
    pub fn on_release(mut self, hook: impl FnOnce() + Send + Sync + 'static) -> WorkspaceGuard {
        self.release_hook = Some(Box::new(hook));
        self
    }
}

impl Drop for WorkspaceGuard {
    fn drop(&mut self) {
        if let Some(hook) = self.release_hook.take() {
            hook();
        }
        self.lock.release(&self.workspace_key);
    }
}
