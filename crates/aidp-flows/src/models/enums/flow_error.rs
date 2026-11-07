use crate::models::enums::RejectReason;
use thiserror::Error;

/// Typed failures of the flow crate. Parsing, evaluation and definition
/// problems are caller errors; `Rejected` is a refused event, not a crash.
#[derive(Debug, Error)]
pub enum FlowError {
    #[error("expression parse error: {0}")]
    Parse(String),
    #[error("expression evaluation error: {0}")]
    Eval(String),
    #[error("flow definition error: {0}")]
    Definition(String),
    #[error("{detail}")]
    Rejected {
        reason: RejectReason,
        detail: String,
    },
}

impl FlowError {
    /// Builds a `Rejected` error from a reason and a human sentence.
    pub fn rejected(reason: RejectReason, detail: impl Into<String>) -> Self {
        FlowError::Rejected {
            reason,
            detail: detail.into(),
        }
    }

    /// The refusal reason, when this error is a refused event.
    pub fn reject_reason(&self) -> Option<RejectReason> {
        match self {
            FlowError::Rejected { reason, .. } => Some(*reason),
            _ => None,
        }
    }
}
