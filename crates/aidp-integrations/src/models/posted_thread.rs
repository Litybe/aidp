use serde::Serialize;

/// A review thread opened for a finding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PostedThread {
    pub finding_id: String,
    pub thread_id: String,
    pub path: Option<String>,
    pub line: Option<i64>,
}
