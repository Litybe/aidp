use serde::{Deserialize, Serialize};

use crate::models::enums::attention_signal_kind::AttentionSignalKind;

/// One stored attention event, keyed by its fingerprint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrAttentionRecord {
    pub fingerprint: String,
    pub repo: String,
    pub number: u64,
    pub signal: AttentionSignalKind,
    pub head_sha: String,
    pub detail: Option<String>,
    pub first_seen: String,
    pub notified: bool,
    pub read: bool,
}
