use crate::models::native_question::NativeQuestion;
use aidp_core::models::assumption::Assumption;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

/// Reply envelope a native stage agent writes to `reply.md`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NativeReplyEnvelope {
    pub status: String,
    pub summary: String,
    #[serde(default)]
    pub outputs: BTreeMap<String, String>,
    #[serde(default)]
    pub signals: Option<Value>,
    #[serde(default)]
    pub follow_ups: Vec<String>,
    #[serde(default)]
    pub assumptions: Vec<Assumption>,
    #[serde(default)]
    pub ask_user_questions: Vec<NativeQuestion>,
}
