use serde::{Deserialize, Serialize};

use crate::models::enums::attention_signal_kind::AttentionSignalKind;

/// One deduplicable event for a pull request, with whether it should notify.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttentionSignal {
    pub kind: AttentionSignalKind,
    pub fingerprint: String,
    pub detail: Option<String>,
    pub notify: bool,
}
