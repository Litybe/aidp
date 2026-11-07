use crate::models::enums::content_part_type::ContentPartType;
use serde::Deserialize;

/// One content part of a transcript message.
#[derive(Debug, Deserialize)]
pub struct TranscriptPart {
    #[serde(rename = "type")]
    pub kind: ContentPartType,
    pub text: Option<String>,
}
