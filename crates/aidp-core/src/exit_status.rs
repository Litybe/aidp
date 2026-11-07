pub use crate::models::enums::{ExitStatus, ALL};
pub use crate::models::TypedExit;

/// Walks an [`anyhow::Error`]'s cause chain and returns the [`ExitStatus`] the
/// first classifiable cause maps to, or `None` when nothing downcasts.
pub fn classify_core(e: &anyhow::Error) -> Option<ExitStatus> {
    for cause in e.chain() {
        if let Some(typed) = cause.downcast_ref::<TypedExit>() {
            return Some(typed.status);
        }
        if let Some(err) = cause.downcast_ref::<crate::Error>() {
            return Some(match err {
                crate::Error::NotFound(_) => ExitStatus::NotFound,
                crate::Error::ScopeViolation(_)
                | crate::Error::Conflict(_)
                | crate::Error::Unauthorized(_) => ExitStatus::PolicyRefusal,
                crate::Error::Config(_) => ExitStatus::Usage,
                crate::Error::Upstream(_) => ExitStatus::UpstreamFailure,
                crate::Error::SchemaViolation(_)
                | crate::Error::Io(_)
                | crate::Error::Yaml(_)
                | crate::Error::Json(_)
                | crate::Error::Other(_) => ExitStatus::Internal,
            });
        }
    }
    None
}

#[cfg(test)]
#[path = "../tests/unit/classify_core_tests.rs"]
mod classify_core_tests;
