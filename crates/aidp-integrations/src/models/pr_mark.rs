use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::pr_thread_mark::PrThreadMark;

/// Local, per-machine marks on one pull request.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrMark {
    #[serde(default)]
    pub seen_sha: Option<String>,
    #[serde(default)]
    pub seen_at: Option<String>,
    #[serde(default)]
    pub ignored: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fix_session: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fix_at: Option<String>,
    /// Review thread id to the fix session it was handed to.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub threads: BTreeMap<String, PrThreadMark>,
}

impl PrMark {
    /// True while the head commit is the one that was marked seen.
    pub fn is_seen(&self, head_sha: &str) -> bool {
        self.seen_sha.as_deref() == Some(head_sha)
    }
}
