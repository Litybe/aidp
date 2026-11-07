use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Files and values a session reads and writes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentJobIo {
    pub stage_dir: PathBuf,
    pub reply_file: PathBuf,
    pub declared_outputs: Vec<String>,
    pub inputs: BTreeMap<String, String>,
    pub upstream: BTreeMap<String, PathBuf>,
}
