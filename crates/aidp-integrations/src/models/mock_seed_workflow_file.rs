use serde::Deserialize;

/// Seed entry: the content of one workflow file.
#[derive(Deserialize)]
pub struct SeedWorkflowFile {
    pub owner: String,
    pub repo: String,
    pub path: String,
    pub content: String,
}
