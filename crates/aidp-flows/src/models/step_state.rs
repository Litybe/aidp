use crate::models::enums::StepStatus;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// Runtime state of one step.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct StepState {
    pub status: StepStatus,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub detail: Option<String>,
    #[serde(default)]
    pub outputs: BTreeMap<String, Value>,
    #[serde(default)]
    pub iteration: u32,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub finished_at: Option<String>,
    #[serde(default)]
    pub phase: Option<String>,
}
