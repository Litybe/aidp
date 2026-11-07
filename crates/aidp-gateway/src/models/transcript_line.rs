use crate::models::enums::stream_message_type::StreamMessageType;
use crate::models::transcript_message::TranscriptMessage;
use serde::Deserialize;

/// One JSON line of a Claude Code transcript file.
#[derive(Debug, Deserialize)]
pub struct TranscriptLine {
    #[serde(rename = "type")]
    pub kind: Option<StreamMessageType>,
    pub message: Option<TranscriptMessage>,
}
