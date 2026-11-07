use serde::{Deserialize, Serialize};

use crate::models::enums::attention_signal_kind::AttentionSignalKind;

/// Payload of the bus event published for a newly notified attention record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttentionEvent {
    pub fingerprint: String,
    pub repo: String,
    pub number: u64,
    pub signal: AttentionSignalKind,
    pub first_seen: String,
}
