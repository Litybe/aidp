use crate::models::enums::pr_fix_phase::PrFixPhase;
use crate::models::enums::pr_fix_reply_check::PrFixReplyCheck;
use serde::{Deserialize, Serialize};

/// Payload of the `pr.fix_session` bus event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrFixSessionEvent {
    pub repo: String,
    pub number: u64,
    pub session_id: String,
    pub phase: PrFixPhase,
    pub threads: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fixed: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub needs_reviewer: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub not_relevant: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unreported: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assumptions: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pushed_sha: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree_kept: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub summary_missing: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub unanswered: Option<Vec<String>>,
    #[serde(default)]
    pub reply_check: PrFixReplyCheck,
}
