use aidp_integrations::models::pr_file::PrFile;
use serde::Serialize;

/// One changed file with whether the viewer's mark still matches its blob.
#[derive(Debug, Serialize)]
pub struct PrFileView {
    #[serde(flatten)]
    pub file: PrFile,
    pub viewed: bool,
}
