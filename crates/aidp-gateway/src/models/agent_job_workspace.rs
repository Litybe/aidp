use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Where the session works and which other directories it may use.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentJobWorkspace {
    pub cwd: PathBuf,
    pub root: PathBuf,
    pub add_dirs: Vec<PathBuf>,
    pub shared_writer_group: String,
    pub isolated: bool,
}
