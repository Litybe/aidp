use crate::models::pr_file::PrFile;
use serde::Serialize;

/// One page of the files of a pull request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PrFilesPage {
    pub page: u32,
    pub per_page: u32,
    pub has_more: bool,
    pub files: Vec<PrFile>,
}
