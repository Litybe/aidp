use serde::Serialize;

/// The flat error shape of the HTTP API: a message and a stable code.
#[derive(Debug, Clone, Serialize)]
pub struct ModelErrorBody {
    pub error: String,
    pub code: String,
}
