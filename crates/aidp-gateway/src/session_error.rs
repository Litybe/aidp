//! Typed session-store failures, carried inside an `anyhow::Result` and
//! recovered by callers via `downcast_ref::<SessionError>()`. Public
//! signatures stay `anyhow::Result` so a caller that does not care keeps
//! using `?` unchanged.

use crate::sessions::SHUTTING_DOWN_ERROR;
use aidp_integrations::{ErrorKind, HasErrorKind};

/// A failure kind an HTTP handler can recover and map to a status code.
#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{SHUTTING_DOWN_ERROR}")]
    ShuttingDown,
    #[error("{0}")]
    Unauthorized(String),
    #[error("{0}")]
    Config(String),
    #[error("{0}")]
    Upstream(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl HasErrorKind for SessionError {
    fn kind(&self) -> ErrorKind {
        match self {
            SessionError::NotFound(_) => ErrorKind::NotFound,
            SessionError::Conflict(_) => ErrorKind::Conflict,
            SessionError::ShuttingDown => ErrorKind::ShuttingDown,
            SessionError::Unauthorized(_) => ErrorKind::Unauthorized,
            SessionError::Config(_) => ErrorKind::Config,
            SessionError::Upstream(_) => ErrorKind::Upstream,
            SessionError::Io(_) => ErrorKind::Io,
        }
    }
}
