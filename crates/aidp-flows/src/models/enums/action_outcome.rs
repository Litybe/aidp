use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// Result of an action process, as the executor classifies it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "status", content = "value", rename_all = "snake_case")]
pub enum ActionOutcome {
    Ok(BTreeMap<String, Value>),
    TechnicalError(String),
    BusinessError(String),
    Skipped {
        reason: String,
    },
    WaitingExternal {
        reason: String,
        #[serde(default)]
        outputs: BTreeMap<String, Value>,
    },
}
