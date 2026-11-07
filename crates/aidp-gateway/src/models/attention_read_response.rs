use serde::{Deserialize, Serialize};

/// Body of `POST /api/prs/attention/read` and payload of the `pr.attention_read` event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttentionReadResponse {
    pub count: usize,
}
