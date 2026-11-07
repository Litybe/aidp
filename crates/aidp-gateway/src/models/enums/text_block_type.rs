use serde::Serialize;

/// The `type` field of a text content block written to `claude`'s stdin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TextBlockType {
    /// A plain text block.
    Text,
}
