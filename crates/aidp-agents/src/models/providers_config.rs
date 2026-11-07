use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::provider_entry::ProviderEntry;

/// The `providers` object of config.json: a `default` id beside one entry per provider id.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProvidersConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    #[serde(flatten)]
    pub entries: BTreeMap<String, ProviderEntry>,
}
