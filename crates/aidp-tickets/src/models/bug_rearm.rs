use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// The bugs of a parent the doctor forgave when it cured an infrastructure cause: they no longer
/// count toward the cap. Kept in the parent's note under `bug_rearm`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BugRearm {
    /// When the cap was re-armed: one re-arm per parent over the guard window.
    pub at: DateTime<Utc>,
    /// Numbers of the bug issues that no longer count.
    pub forgiven: Vec<u64>,
}
