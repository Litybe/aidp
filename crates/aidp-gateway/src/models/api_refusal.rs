use super::enums::ApiStatus;
use super::ApiError;

/// A launch or reply refused with a status chosen by the workspace or attachment checks.
#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct ApiRefusal {
    pub status: ApiStatus,
    pub message: String,
}

impl From<ApiError> for ApiRefusal {
    fn from(error: ApiError) -> Self {
        ApiRefusal {
            status: error.status,
            message: error.message,
        }
    }
}
