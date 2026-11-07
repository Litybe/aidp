use serde::{Deserialize, Serialize};

/// A human approval point on a stage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GateSpec {
    pub approved: bool,
}
