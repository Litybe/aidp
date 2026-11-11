use serde::{Deserialize, Serialize};

/// Body of a dispatcher pause write: the wanted state and who asked.
#[derive(Debug, Serialize, Deserialize)]
pub struct DispatcherPauseRequest {
    /// Absent means pause: `POST /dispatcher/pause` with `{}` (or no body) pauses.
    #[serde(default = "default_paused")]
    pub paused: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
}

fn default_paused() -> bool {
    true
}

#[cfg(test)]
#[path = "../../tests/unit/dispatcher_pause_request_tests.rs"]
mod tests;
