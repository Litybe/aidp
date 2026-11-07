use serde::Deserialize;

/// The `message` object of a transcript line.
#[derive(Debug, Deserialize)]
pub struct TranscriptMessage {
    pub content: Option<serde_json::Value>,
}
