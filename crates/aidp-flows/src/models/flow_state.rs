use crate::models::enums::FlowStatus;
use crate::models::{FlowDef, Frame, StepState};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};

/// Persisted state of one run. `def` is the frozen copy of the definition
/// the run started with, version included.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FlowState {
    pub key: String,
    pub def: FlowDef,
    pub status: FlowStatus,
    #[serde(default)]
    pub current: Option<String>,
    #[serde(default)]
    pub steps: BTreeMap<String, StepState>,
    #[serde(default)]
    pub effects_applied: BTreeSet<String>,
    #[serde(default)]
    pub inserted_ci: Vec<String>,
    #[serde(default)]
    pub stack: Vec<Frame>,
    #[serde(default)]
    pub ticket: Value,
    #[serde(default)]
    pub inputs: BTreeMap<String, Value>,
    #[serde(default)]
    pub settings: BTreeMap<String, Value>,
    #[serde(default)]
    pub pause_requested: bool,
    #[serde(default)]
    pub seq: u64,
    #[serde(default)]
    pub manual_count: u32,
    #[serde(default)]
    pub manual_publish: BTreeSet<String>,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub ended_at: Option<String>,
}

impl FlowState {
    /// A fresh, unstarted run of `def` for ticket `key`.
    pub fn new(key: impl Into<String>, def: FlowDef) -> Self {
        FlowState {
            key: key.into(),
            inputs: def.inputs.clone(),
            settings: def.settings.clone(),
            def,
            status: FlowStatus::Running,
            current: None,
            steps: BTreeMap::new(),
            effects_applied: BTreeSet::new(),
            inserted_ci: Vec::new(),
            stack: Vec::new(),
            ticket: Value::Null,
            pause_requested: false,
            seq: 0,
            manual_count: 0,
            manual_publish: BTreeSet::new(),
            started_at: None,
            ended_at: None,
        }
    }

    /// True until the `Start` event has been applied.
    pub fn is_fresh(&self) -> bool {
        self.stack.is_empty() && self.steps.is_empty() && self.started_at.is_none()
    }
}
