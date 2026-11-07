use serde::Serialize;

use super::approved_gates_args::ApprovedGatesArgs;

/// Resume payload printed by `aidp gate approve`, keyed like the `Workflow` tool arguments.
/// Keys are camelCase: `scriptPath`, `resumeFromRunId`, `args`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GateApproveResume<'a> {
    pub args: ApprovedGatesArgs<'a>,
    pub resume_from_run_id: &'a Option<String>,
    pub script_path: String,
}
