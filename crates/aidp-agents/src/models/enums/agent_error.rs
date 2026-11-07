use thiserror::Error;

use super::provider_origin::ProviderOrigin;

/// Failures of provider resolution, startup and control.
#[derive(Debug, Error)]
pub enum AgentError {
    #[error("this provider does not support the requested session control")]
    UnsupportedControl,
    #[error("unknown agent provider `{name}` (from {origin})")]
    UnknownProvider {
        name: String,
        origin: ProviderOrigin,
    },
    #[error("agent provider `{provider}` needs `{program}` but it was not found on PATH")]
    BinaryNotFound { provider: String, program: String },
    #[error("failed to start agent: {0}")]
    Spawn(String),
    #[error("invalid providers config: {0}")]
    Config(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}
