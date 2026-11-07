use aidp_flows::FlowStatus;
use serde::{Deserialize, Serialize};

/// One run in the daemon's index.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndexEntry {
    pub key: String,
    pub repo: String,
    #[serde(default)]
    pub service: Option<String>,
    pub flow: String,
    pub status: FlowStatus,
    #[serde(default)]
    pub current: Option<String>,
    pub updated_at: String,
}
