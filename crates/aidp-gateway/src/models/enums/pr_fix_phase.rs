use serde::{Deserialize, Serialize};

/// Lifecycle phase carried by the `pr.fix_session` event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PrFixPhase {
    Started,
    Ended,
}
