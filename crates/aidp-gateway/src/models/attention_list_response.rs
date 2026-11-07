use serde::{Deserialize, Serialize};

use crate::models::pr_attention_record::PrAttentionRecord;

/// Body of `GET /api/prs/attention`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttentionListResponse {
    pub items: Vec<PrAttentionRecord>,
}
