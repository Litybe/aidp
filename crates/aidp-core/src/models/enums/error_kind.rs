/// Shared classification for the crate error types (`SessionError`, `TicketError`,
/// `IntegrationError`) that otherwise redeclare the same variant set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// The requested resource does not exist.
    NotFound,
    /// The request conflicts with the resource's current state.
    Conflict,
    /// The caller is not authorized to perform the action.
    Unauthorized,
    /// The configuration is missing or invalid.
    Config,
    /// A call to an upstream system failed.
    Upstream,
    /// A local I/O operation failed.
    Io,
    /// The owning process is shutting down.
    ShuttingDown,
}

/// Implemented by a crate's own error type to expose its `ErrorKind`.
pub trait HasErrorKind {
    /// The kind this error value classifies as.
    fn kind(&self) -> ErrorKind;
}

impl ErrorKind {
    /// The stable wire slug for this kind, unchanged across crates.
    pub const fn as_str(&self) -> &'static str {
        match self {
            ErrorKind::NotFound => "not_found",
            ErrorKind::Conflict => "conflict",
            ErrorKind::Unauthorized => "unauthorized",
            ErrorKind::Config => "config",
            ErrorKind::Upstream => "upstream",
            ErrorKind::Io => "io",
            ErrorKind::ShuttingDown => "shutting_down",
        }
    }
}
