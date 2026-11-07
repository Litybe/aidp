use serde::Serialize;

/// Payload of the paused-changed bus event, published on each effective change.
#[derive(Debug, Serialize)]
pub struct DispatcherPausedChangedEvent {
    pub paused: bool,
    pub actor: String,
    pub at: String,
}
