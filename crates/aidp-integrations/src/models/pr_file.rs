use serde::Serialize;

/// One file of a pull request diff; `patch` is absent when missing or over the size cap.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrFile {
    pub filename: String,
    pub status: String,
    pub additions: u64,
    pub deletions: u64,
    pub patch: Option<String>,
    pub previous_filename: Option<String>,
    pub sha: Option<String>,
    pub too_large: bool,
}
