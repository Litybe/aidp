use serde::{Deserialize, Serialize};

/// State a comment point can be set to; `None` clears it.
#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum PointState {
    Ignored,
    None,
}
