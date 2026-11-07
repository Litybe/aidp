use serde_json::Value;
use std::collections::BTreeMap;

pub(crate) const ID_SEPARATOR: &str = ":";

/// An action to run on a worker thread.
pub(crate) struct ActionJob {
    pub(crate) key: String,
    pub(crate) repo: String,
    pub(crate) effect_id: String,
    pub(crate) name: String,
    pub(crate) with: BTreeMap<String, Value>,
}

impl ActionJob {
    /// Identifies the job among all runs: effect ids repeat from one run to the next.
    pub(crate) fn inflight_id(&self) -> String {
        format!("{}{ID_SEPARATOR}{}", self.key, self.effect_id)
    }
}
