use serde::{Deserialize, Serialize};

/// Where an incident of the doctor stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DoctorIncidentStatus {
    /// Detected; the investigation has not started yet.
    Detected,
    /// The investigation run is going.
    Investigating,
    /// The allowed action ran and succeeded.
    Fixed,
    /// The allowed action ran and failed.
    ActionFailed,
    /// The diagnosis asks for a human, or its confidence is too low.
    NeedsHuman,
    /// The diagnosis proposed an action outside the allowed list, or one whose precondition fails.
    Refused,
    /// The diagnosis found nothing to do.
    NoAction,
    /// The investigation run failed or gave no readable diagnosis. Never retried.
    InvestigationFailed,
    /// The ticket already had its investigations of the last 24 hours.
    Capped,
}

impl DoctorIncidentStatus {
    /// The wire value, identical to the serialized form.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Detected => "detected",
            Self::Investigating => "investigating",
            Self::Fixed => "fixed",
            Self::ActionFailed => "action_failed",
            Self::NeedsHuman => "needs_human",
            Self::Refused => "refused",
            Self::NoAction => "no_action",
            Self::InvestigationFailed => "investigation_failed",
            Self::Capped => "capped",
        }
    }

    /// True once nothing more happens to the incident.
    pub const fn is_closed(self) -> bool {
        !matches!(self, Self::Detected | Self::Investigating)
    }
}
