use crate::models::constants::flow_external::{
    OUTCOME_BUSINESS_ERROR, OUTCOME_OK, OUTCOME_PENDING, OUTCOME_SATISFIED, OUTCOME_SKIPPED,
    OUTCOME_TECHNICAL_ERROR, OUTCOME_WAITING_EXTERNAL,
};

/// The outcome an external caller reports for a job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalOutcome {
    Ok,
    Skipped,
    WaitingExternal,
    TechnicalError,
    BusinessError,
    Satisfied,
    Pending,
}

impl ExternalOutcome {
    /// The outcome named by `text`, `None` when unknown.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            OUTCOME_OK => Some(ExternalOutcome::Ok),
            OUTCOME_SKIPPED => Some(ExternalOutcome::Skipped),
            OUTCOME_WAITING_EXTERNAL => Some(ExternalOutcome::WaitingExternal),
            OUTCOME_TECHNICAL_ERROR => Some(ExternalOutcome::TechnicalError),
            OUTCOME_BUSINESS_ERROR => Some(ExternalOutcome::BusinessError),
            OUTCOME_SATISFIED => Some(ExternalOutcome::Satisfied),
            OUTCOME_PENDING => Some(ExternalOutcome::Pending),
            _ => None,
        }
    }

    /// True for the two outcomes of a wait probe.
    pub const fn is_probe(self) -> bool {
        matches!(self, ExternalOutcome::Satisfied | ExternalOutcome::Pending)
    }
}
