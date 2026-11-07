use crate::models::enums::native_stage_status::NativeStageStatus;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// One row of a native run: a stage, or a plan task or shard named `<stage>#<id>`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeStageView {
    pub name: String,
    pub status: NativeStageStatus,
    /// Why the row is skipped, failed or cancelled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Sessions launched for the row, one per attempt or repair.
    #[serde(default)]
    pub attempts: u32,
    /// True once the row was relaunched after an interruption; a second one fails it.
    #[serde(default)]
    pub resumed: bool,
    #[serde(default)]
    pub session_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    /// `signals` of the accepted reply, read by conditions and loops.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signals: Option<Value>,
}

impl NativeStageView {
    /// A row that has not started.
    pub fn pending(name: &str) -> Self {
        NativeStageView {
            name: name.to_string(),
            status: NativeStageStatus::Pending,
            reason: None,
            attempts: 0,
            resumed: false,
            session_ids: Vec::new(),
            started_at: None,
            finished_at: None,
            cost_usd: None,
            signals: None,
        }
    }
}
