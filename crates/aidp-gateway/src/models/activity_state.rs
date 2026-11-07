use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Thread activity remembered between two polls of the reviews probe.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ActivityState {
    pub seen: u64,
    pub thread_ids: Vec<String>,
    pub pending_new: u64,
    pub last_event_ms: i64,
}

impl ActivityState {
    /// Reads the remembered state, falling back to the empty state when absent or malformed.
    pub fn from_last(value: Option<&Value>) -> Self {
        value
            .cloned()
            .and_then(|stored| serde_json::from_value(stored).ok())
            .unwrap_or_default()
    }

    /// Serializes the state for the next poll.
    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }
}
