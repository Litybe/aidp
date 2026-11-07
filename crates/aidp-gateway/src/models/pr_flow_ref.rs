use serde::Serialize;

/// The active ticket flow attached to a pull request row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PrFlowRef {
    pub key: String,
    pub phase: String,
}
