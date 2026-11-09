use serde::{Deserialize, Serialize};

/// What made the doctor open an incident.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DoctorTrigger {
    /// A flow ended `failed`.
    FlowFailed,
    /// A flow ended `escalated` for a reason a human did not choose.
    FlowEscalated,
    /// A native run stopped `paused` (needs review, blocked, rejected reply, write out of scope).
    RunPaused,
    /// A step still `running` while its run ended, or with no live run, session or process.
    StepStuck,
    /// A step `waiting_external` for too long.
    WaitingTooLong,
}

impl DoctorTrigger {
    /// The wire value, identical to the serialized form.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FlowFailed => "flow_failed",
            Self::FlowEscalated => "flow_escalated",
            Self::RunPaused => "run_paused",
            Self::StepStuck => "step_stuck",
            Self::WaitingTooLong => "waiting_too_long",
        }
    }
}
