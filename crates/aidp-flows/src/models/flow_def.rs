use crate::models::enums::{FlowError, WorkspaceMode};
use crate::models::{FlowEntry, Step};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// A flow definition, as written in `templates/flows/`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FlowDef {
    pub id: String,
    pub version: u32,
    #[serde(default)]
    pub inputs: BTreeMap<String, Value>,
    #[serde(default)]
    pub settings: BTreeMap<String, Value>,
    #[serde(default)]
    pub blocks: BTreeMap<String, Vec<Step>>,
    #[serde(default)]
    pub steps: Vec<Step>,
    /// `project`: the steps run in the main checkout of the project (no worktree) and the run
    /// holds the project's `unity-editor` resource until `git.restore` gives the checkout back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<WorkspaceMode>,
    /// Named entries a run can be started from by hand (a forced move of its ticket).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub entries: BTreeMap<String, FlowEntry>,
}

impl FlowDef {
    /// Parses a definition from YAML text.
    pub fn from_yaml(text: &str) -> Result<FlowDef, FlowError> {
        serde_yaml::from_str(text).map_err(|error| FlowError::Definition(error.to_string()))
    }
}
