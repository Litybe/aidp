use serde::Deserialize;

/// The `type` field of one content part of a transcript message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContentPartType {
    /// A plain text part.
    Text,
    /// Any other part kind (tool use, image, thinking).
    #[serde(other)]
    Other,
}
