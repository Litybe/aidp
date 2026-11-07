use serde::{Deserialize, Serialize};

/// One selected review thread: its anchor, diff hunk and `(author, body)` conversation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrFixThreadInput {
    pub id: String,
    pub path: String,
    #[serde(default)]
    pub line: Option<u64>,
    #[serde(default)]
    pub diff_hunk: String,
    #[serde(default)]
    pub conversation: Vec<(String, String)>,
}
