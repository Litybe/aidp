use crate::models::enums::native_run_status::NativeRunStatus;
use crate::models::enums::native_stage_status::NativeStageStatus;
use serde::Serialize;

/// Bus payload of `run.started`, `run.stage_changed`, `run.waiting_gate` and `run.ended`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NativeRunEvent {
    pub run_id: String,
    pub template_id: String,
    pub status: NativeRunStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stage_status: Option<NativeStageStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}
