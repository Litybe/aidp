use serde::{Deserialize, Serialize};

/// Where the embedding model of the daemon stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelState {
    Stopped,
    Loading,
    Running,
    Error,
}

impl ModelState {
    /// The wire name of the state.
    pub const fn as_str(self) -> &'static str {
        match self {
            ModelState::Stopped => "stopped",
            ModelState::Loading => "loading",
            ModelState::Running => "running",
            ModelState::Error => "error",
        }
    }
}
