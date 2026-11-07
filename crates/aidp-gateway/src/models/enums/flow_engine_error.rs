use aidp_core::models::enums::{ErrorKind, HasErrorKind};
use serde_json::Value;

/// Failures of the engine that map to a precise HTTP answer.
#[derive(Debug, thiserror::Error)]
pub enum FlowEngineError {
    #[error("{0}")]
    NotFound(String),
    /// The request does not fit the run's state. `existing` is the run
    /// already holding the ticket, when that is the cause.
    #[error("{message}")]
    Conflict {
        message: String,
        existing: Option<Value>,
    },
    #[error("{0}")]
    Config(String),
}

impl HasErrorKind for FlowEngineError {
    fn kind(&self) -> ErrorKind {
        match self {
            FlowEngineError::NotFound(_) => ErrorKind::NotFound,
            FlowEngineError::Conflict { .. } => ErrorKind::Conflict,
            FlowEngineError::Config(_) => ErrorKind::Config,
        }
    }
}
