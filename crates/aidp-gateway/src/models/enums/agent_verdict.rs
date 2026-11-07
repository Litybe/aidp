use crate::models::native_reply_envelope::NativeReplyEnvelope;
use serde::{Deserialize, Serialize};

/// What aidp decides about the result a host posted for an agent job.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentVerdict {
    Accepted(NativeReplyEnvelope),
    /// The reply was invalid: `instruction` is the text the repair job appends to the prompt.
    Repair {
        error: String,
        instruction: String,
    },
    Failed(String),
    Cancelled,
}
