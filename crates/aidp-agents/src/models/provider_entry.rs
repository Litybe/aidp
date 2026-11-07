use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Per-provider settings: executable, fixed args and tier-to-model mapping.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub program: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub models: BTreeMap<String, String>,
}
