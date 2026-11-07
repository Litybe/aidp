use serde::Serialize;

/// Error body carrying one message.
#[derive(Serialize)]
pub struct MockError {
    pub error: String,
}
