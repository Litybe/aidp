use serde::{Deserialize, Serialize};

/// Which attention records to mark read: some fingerprints, or all of them.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttentionReadRequest {
    pub fingerprints: Option<Vec<String>>,
    pub all: Option<bool>,
}
