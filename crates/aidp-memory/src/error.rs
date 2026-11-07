use aidp_core::models::enums::{ErrorKind, HasErrorKind};
use thiserror::Error;

/// Typed failures of the memory crate, recovered by the CLI through `downcast_ref`.
#[derive(Debug, Error)]
pub enum MemoryError {
    #[error("{0}")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("memory database: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("{0}")]
    Json(#[from] serde_json::Error),
}

/// Result of every fallible call of this crate.
pub type Result<T> = std::result::Result<T, MemoryError>;

impl HasErrorKind for MemoryError {
    fn kind(&self) -> ErrorKind {
        match self {
            MemoryError::NotFound(_) => ErrorKind::NotFound,
            MemoryError::Conflict(_) => ErrorKind::Conflict,
            MemoryError::Invalid(_) => ErrorKind::Config,
            MemoryError::Io(_) | MemoryError::Database(_) | MemoryError::Json(_) => ErrorKind::Io,
        }
    }
}
