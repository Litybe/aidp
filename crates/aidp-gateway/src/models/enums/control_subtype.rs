use serde::{Deserialize, Serialize};

/// The `subtype` of a control request or response.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlSubtype {
    CanUseTool,
    Success,
}
