//! Resource caps of the flow configuration.

use crate::models::constants::flow_repo::DEFAULT_WORKTREE_CAP;
use serde::{Deserialize, Serialize};

/// Upper bounds on what a flow may hold at once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FlowCapsConfig {
    pub worktrees: u32,
}

impl Default for FlowCapsConfig {
    fn default() -> Self {
        Self {
            worktrees: DEFAULT_WORKTREE_CAP,
        }
    }
}
