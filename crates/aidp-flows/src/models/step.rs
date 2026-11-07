use crate::models::enums::{OnError, StepKind};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// One step of a flow. `use_` names an action, a wait, a pipeline
/// template or a block.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub id: String,
    #[serde(default)]
    pub kind: StepKind,
    #[serde(default)]
    pub phase: Option<String>,
    #[serde(default)]
    pub when: Option<String>,
    #[serde(default)]
    pub skip_on: Vec<String>,
    #[serde(default)]
    pub pushes: bool,
    /// Daemon resource the step holds while it runs, e.g. `unity-editor`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource: Option<String>,
    #[serde(rename = "use", default)]
    pub use_: Option<String>,
    #[serde(default)]
    pub then: Option<String>,
    #[serde(default)]
    pub on_error: OnError,
    #[serde(default)]
    pub with: BTreeMap<String, Value>,
    #[serde(default)]
    pub max: Option<u32>,
    #[serde(default)]
    pub until: Option<String>,
    #[serde(default)]
    pub body: Vec<Step>,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub choices: Vec<String>,
}

impl Step {
    /// A step with only its id and kind set.
    pub fn new(id: impl Into<String>, kind: StepKind) -> Self {
        Step {
            id: id.into(),
            kind,
            ..Step::default()
        }
    }

    /// True when this step or any step nested in its body is a `human` step.
    pub fn contains_human(&self) -> bool {
        self.kind == StepKind::Human || self.body.iter().any(Step::contains_human)
    }
}
