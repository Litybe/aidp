use crate::models::pr_file_view::PrFileView;
use serde::Serialize;

/// Reply of `GET /api/prs/files`: one page of files plus those changed since the viewer's marks.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrFilesResponse {
    pub page: u32,
    pub per_page: u32,
    pub has_more: bool,
    pub files: Vec<PrFileView>,
    pub since_last_review: Vec<String>,
}
