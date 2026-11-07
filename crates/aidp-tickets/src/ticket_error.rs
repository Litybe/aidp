use aidp_core::models::enums::{ErrorKind, HasErrorKind};
use thiserror::Error;

/// Typed ticket-store failures, carried inside an `anyhow::Result` and
/// recovered by callers via `downcast_ref::<TicketError>()`. Public
/// signatures stay `anyhow::Result` so a caller that does not care keeps
/// using `?` unchanged.
#[derive(Debug, Error)]
pub enum TicketError {
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    Unauthorized(String),
    #[error("{0}")]
    Config(String),
    #[error("{0}")]
    Upstream(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl HasErrorKind for TicketError {
    fn kind(&self) -> ErrorKind {
        match self {
            TicketError::NotFound(_) => ErrorKind::NotFound,
            TicketError::Conflict(_) => ErrorKind::Conflict,
            TicketError::Unauthorized(_) => ErrorKind::Unauthorized,
            TicketError::Config(_) => ErrorKind::Config,
            TicketError::Upstream(_) => ErrorKind::Upstream,
            TicketError::Io(_) => ErrorKind::Io,
        }
    }
}
