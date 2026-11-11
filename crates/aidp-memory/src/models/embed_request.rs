use serde::{Deserialize, Serialize};

/// Body of `POST /api/embed`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmbedRequest {
    pub texts: Vec<String>,
}
