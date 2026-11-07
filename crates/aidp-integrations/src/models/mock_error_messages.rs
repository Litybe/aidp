use serde::Serialize;

/// Jira-style error body listing human messages.
#[derive(Serialize)]
pub struct MockErrorMessages {
    #[serde(rename = "errorMessages")]
    pub error_messages: Vec<String>,
}

impl MockErrorMessages {
    /// A body carrying one message.
    pub fn single(message: impl Into<String>) -> Self {
        Self {
            error_messages: vec![message.into()],
        }
    }
}
