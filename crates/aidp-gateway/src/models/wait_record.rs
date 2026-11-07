use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// A wait the executor polls.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WaitRecord {
    pub step: String,
    pub probe: String,
    pub with: BTreeMap<String, Value>,
    pub started_ms: i64,
    /// Probe memory from the last poll; absent in records stored before it existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<Value>,
}
