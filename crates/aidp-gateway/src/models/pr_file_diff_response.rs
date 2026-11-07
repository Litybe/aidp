use serde::Serialize;

/// Reply of `GET /api/prs/file-diff`: the full unified diff of one file.
#[derive(Debug, Serialize)]
pub struct PrFileDiffResponse {
    pub path: String,
    pub diff: String,
}
