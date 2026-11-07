use serde::{Deserialize, Serialize};

const BLOCKER_CYCLE: &str = "blocker cycle";

/// Why a launch went ahead with a warning attached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LaunchDetail {
    #[serde(rename = "blocker cycle")]
    BlockerCycle,
}

impl LaunchDetail {
    /// The wire string of this detail.
    pub fn as_str(self) -> &'static str {
        match self {
            LaunchDetail::BlockerCycle => BLOCKER_CYCLE,
        }
    }
}
