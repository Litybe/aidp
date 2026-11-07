use crate::models::enums::Source;
use serde::{Deserialize, Serialize};

/// One indexed passage of a project file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Chunk {
    pub id: String,
    pub project: String,
    pub source: Source,
    pub path: String,
    pub line_start: u32,
    pub line_end: u32,
    pub title: String,
    pub text: String,
    pub hash: String,
    pub mtime: i64,
    pub meta: serde_json::Value,
}
