use serde::Serialize;

/// Envelope printed by `aidp validate --all`.
/// Fields are declared in alphabetical order to match the wire output.
#[derive(Serialize)]
pub struct ValidateAllSummary {
    pub failed: usize,
    pub ok: bool,
    pub results: Vec<serde_json::Value>,
    pub total: usize,
}
