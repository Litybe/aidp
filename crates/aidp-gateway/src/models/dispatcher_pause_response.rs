use serde::{Deserialize, Serialize};

/// Reply of a dispatcher pause write: the effective paused state.
#[derive(Debug, Serialize, Deserialize)]
pub struct DispatcherPauseResponse {
    pub paused: bool,
}
