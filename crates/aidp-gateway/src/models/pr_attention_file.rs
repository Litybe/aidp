use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use crate::models::pr_attention_record::PrAttentionRecord;

/// On-disk shape of the attention store.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrAttentionFile {
    #[serde(default)]
    pub records: BTreeMap<String, PrAttentionRecord>,
}
