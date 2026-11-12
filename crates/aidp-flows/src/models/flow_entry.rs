use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// A place a run can be started from by hand, skipping the top-level steps before `step`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FlowEntry {
    /// The top-level step the run lands on.
    pub step: String,
    /// Top-level steps before `step` that still run (workspace preparation).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keep: Vec<String>,
    /// Outputs assumed for skipped steps, so the conditions after `step` hold.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub outputs: BTreeMap<String, BTreeMap<String, Value>>,
}
