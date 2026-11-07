use serde::{Deserialize, Serialize};

/// Body of a dispatcher pause write: the wanted state and who asked.
#[derive(Debug, Serialize, Deserialize)]
pub struct DispatcherPauseRequest {
    pub paused: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
}
