use crate::models::constants::pr_status_keys::{
    KEY_CHECKS, KEY_GATE, KEY_HEAD, KEY_REVIEW, KEY_SHA, KEY_THREADS,
};
use serde_json::Value;

/// Read access to the status of the pull request of a ticket key.
pub trait PrStatusSource: Send + Sync {
    /// The full status document of the key's PR, `None` when it has no PR or cannot be read.
    fn status(&self, key: &str) -> Option<Value>;

    /// Reads a fresh snapshot when a wait must not conclude from cached threads.
    fn refresh(&self, key: &str) -> Option<Value> {
        self.status(key)
    }

    fn head_sha(&self, key: &str) -> Option<String> {
        let status = self.status(key)?;
        let sha = status.get(KEY_HEAD)?.get(KEY_SHA)?.as_str()?;
        Some(sha.to_string())
    }

    fn checks(&self, key: &str) -> Option<Value> {
        self.status(key)?.get(KEY_CHECKS).cloned()
    }

    /// The review report: decision, approvals and reviewers.
    fn review(&self, key: &str) -> Option<Value> {
        self.status(key)?.get(KEY_REVIEW).cloned()
    }

    fn threads(&self, key: &str) -> Option<Value> {
        self.status(key)?.get(KEY_THREADS).cloned()
    }

    /// The merge gate, which also tells whether the PR is approved.
    fn gate(&self, key: &str) -> Option<Value> {
        self.status(key)?.get(KEY_GATE).cloned()
    }
}
