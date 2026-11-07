//! A decision a stage left to a human instead of taking it itself.

use serde::{Deserialize, Serialize};

/// One escalated decision a stage reported in `signals.escalations`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Escalation {
    #[serde(default)]
    pub stage: String,
    pub id: String,
    pub decision: String,
    pub reason: String,
}
