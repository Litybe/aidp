//! Typed error kinds shared across the Jira, Confluence, GitHub and CI
//! integrations. Callers that need to branch on the failure kind recover it
//! from the `anyhow::Error` chain with `downcast_ref::<IntegrationError>()`;
//! every function here keeps returning `anyhow::Result` so its `?` call
//! sites are unchanged.

/// A failure kind an integration caller can recover and branch on.
#[derive(Debug, thiserror::Error)]
pub enum IntegrationError {
    #[error("{0} not found")]
    NotFound(String),
    #[error("{0}")]
    Conflict(String),
    #[error("{0}")]
    Unauthorized(String),
    #[error("environment variable {var} is not set: {remedy}")]
    Config {
        var: &'static str,
        remedy: &'static str,
    },
    #[error("{0}")]
    Upstream(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl aidp_core::models::enums::HasErrorKind for IntegrationError {
    fn kind(&self) -> aidp_core::models::enums::ErrorKind {
        match self {
            IntegrationError::NotFound(_) => aidp_core::models::enums::ErrorKind::NotFound,
            IntegrationError::Conflict(_) => aidp_core::models::enums::ErrorKind::Conflict,
            IntegrationError::Unauthorized(_) => aidp_core::models::enums::ErrorKind::Unauthorized,
            IntegrationError::Config { .. } => aidp_core::models::enums::ErrorKind::Config,
            IntegrationError::Upstream(_) => aidp_core::models::enums::ErrorKind::Upstream,
            IntegrationError::Io(_) => aidp_core::models::enums::ErrorKind::Io,
        }
    }
}
