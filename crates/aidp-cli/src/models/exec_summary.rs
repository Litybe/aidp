use aidp_gateway::models::enums::native_run_status::NativeRunStatus;
use serde::Serialize;

/// Final summary printed by `aidp run exec --foreground`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecSummary {
    pub run_id: String,
    pub status: NativeRunStatus,
    pub reason: Option<String>,
}
