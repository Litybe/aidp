use crate::models::enums::ApiStatus;

/// A status/code/message triple recovered from a typed error chain, so a
/// handler can map its failure to the right HTTP response without a string
/// compare.
#[derive(Debug)]
pub struct ApiError {
    pub status: ApiStatus,
    pub message: String,
}
