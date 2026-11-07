use serde::Deserialize;

use crate::models::mock_seed_workflow_file::SeedWorkflowFile;

/// Body of the seed route.
#[derive(Deserialize, Default)]
pub struct SeedRequest {
    #[serde(default)]
    pub workflow_files: Vec<SeedWorkflowFile>,
}
