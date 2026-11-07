use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// A manual request made outside the flow's own steps.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ManualRequest {
    Action {
        name: String,
        with: BTreeMap<String, Value>,
    },
    Pipeline {
        template: String,
        #[serde(default)]
        publish: bool,
        with: BTreeMap<String, Value>,
    },
}
