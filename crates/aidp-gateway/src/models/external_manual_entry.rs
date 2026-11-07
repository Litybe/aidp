use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// A manual action held for the external caller, keyed by its effect id in the runtime.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExternalManualEntry {
    pub step: String,
    pub name: String,
    pub with: BTreeMap<String, Value>,
    pub created_at: String,
}
