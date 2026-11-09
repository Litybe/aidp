use crate::models::enums::DoctorAction;
use serde::{Deserialize, Serialize};

/// What the daemon did with the action of a diagnosis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorActionOutcome {
    pub action: DoctorAction,
    /// `<incident id>:action`: an action is executed at most once per incident.
    pub idempotency_key: String,
    pub ok: bool,
    pub detail: String,
    pub at: String,
}
