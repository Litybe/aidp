use serde::Serialize;

/// GitHub-style error body carrying one message.
#[derive(Serialize)]
pub struct MockMessage {
    pub message: String,
}
