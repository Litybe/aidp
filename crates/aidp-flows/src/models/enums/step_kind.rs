use serde::{Deserialize, Serialize};

/// Nature of a step.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    #[default]
    Action,
    Pipeline,
    Wait,
    Human,
    Loop,
}
