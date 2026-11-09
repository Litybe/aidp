use crate::models::doctor_action_outcome::DoctorActionOutcome;
use crate::models::doctor_candidate::DoctorCandidate;
use crate::models::doctor_diagnosis::DoctorDiagnosis;
use crate::models::enums::DoctorIncidentStatus;
use serde::{Deserialize, Serialize};

/// One incident of the doctor: what broke, the investigation, the action and its result.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorIncident {
    #[serde(flatten)]
    pub candidate: DoctorCandidate,
    pub status: DoctorIncidentStatus,
    pub detected_at: String,
    pub updated_at: String,
    /// Native run of the `flow-doctor` investigation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub investigation_run_id: Option<String>,
    /// Epoch milliseconds the investigation started, for the per-ticket cap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub investigated_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnosis: Option<DoctorDiagnosis>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<DoctorActionOutcome>,
    /// Why no action ran, or what went wrong.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default)]
    pub commented: bool,
}
