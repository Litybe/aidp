use crate::models::enums::session_outcome::SessionOutcome;
use aidp_core::run::StageUsage;

/// A finished [`SessionOutcome`] with the usage and duration of the sessions behind it.
#[derive(Debug)]
pub struct SessionRun {
    pub outcome: SessionOutcome,
    pub usage: Option<StageUsage>,
    pub duration_ms: u64,
}
