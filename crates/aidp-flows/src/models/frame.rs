use crate::models::enums::FrameKind;
use crate::models::Step;
use serde::{Deserialize, Serialize};

/// One level of the execution stack: a step list and the next index.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Frame {
    pub kind: FrameKind,
    pub prefix: String,
    #[serde(default)]
    pub owner: Option<String>,
    pub steps: Vec<Step>,
    pub index: usize,
}
