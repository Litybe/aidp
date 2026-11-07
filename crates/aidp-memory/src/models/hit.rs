use crate::models::enums::Source;
use serde::{Deserialize, Serialize};

/// A ranked search result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hit {
    pub id: String,
    pub source: Source,
    pub path: String,
    pub line_start: u32,
    pub line_end: u32,
    pub title: String,
    pub score: f64,
    pub text: String,
    /// `bm25` for a direct match, `graph` for a neighbour of a matched symbol.
    pub via: String,
}

impl Hit {
    /// The citation `path:L<start>-L<end>`.
    pub fn citation(&self) -> String {
        format!("{}:L{}-L{}", self.path, self.line_start, self.line_end)
    }
}
