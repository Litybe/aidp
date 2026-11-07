use aidp_core::run::GateDecisionKind;
use serde::Serialize;

/// `{"ok": true}` printed by `aidp scope check` when the path is in scope.
/// Serialized with `output::to_spaced_json`, which is the wire format the driver reads.
#[derive(Serialize)]
pub struct OkEnvelope {
    pub ok: bool,
}

/// Anchor printed by `aidp run record-resume`. Field order is the wire order.
#[derive(Serialize)]
pub struct RecordResumeEnvelope<'a> {
    pub ok: bool,
    pub workflow_run_id: &'a Option<String>,
    pub approved_gates: Vec<String>,
    pub pending_gate: &'a Option<String>,
    pub crossed_boundaries: &'a [String],
}

/// Decision echoed by `aidp gate abort` and `aidp gate pending`. Field order is the wire order.
#[derive(Serialize)]
pub struct GateDecisionEnvelope<'a> {
    pub ok: bool,
    pub gate: &'a str,
    pub decision: GateDecisionKind,
}

/// Report printed by `aidp setup merge-hooks --dry-run`.
/// Fields are declared in alphabetical order to match the wire output.
#[derive(Serialize)]
pub struct MergeHooksDryRun<'a, Preview: Serialize> {
    pub dry_run: bool,
    pub hooks_added: usize,
    pub preview: &'a Preview,
    pub target: String,
}

/// Result printed by `aidp setup merge-hooks` after writing the target.
/// Fields are declared in alphabetical order to match the wire output.
#[derive(Serialize)]
pub struct MergeHooksDone {
    pub hooks_added: usize,
    pub ok: bool,
    pub target: String,
}

/// Result printed by `aidp setup agents`.
/// Fields are declared in alphabetical order to match the wire output.
#[derive(Serialize)]
pub struct SetupAgentsDone {
    pub dest: String,
    pub force: bool,
    pub ok: bool,
    pub written: Vec<String>,
}

/// Pass, fail and skip tallies of a merged findings file.
#[derive(Serialize)]
pub struct ChecksSummary {
    pub fail: u32,
    pub pass: u32,
    pub skip: u32,
}

/// Envelope printed by `aidp findings merge`, naming the merge log and degraded axes.
/// Fields are declared in alphabetical order to match the wire output.
#[derive(Serialize)]
pub struct FindingsMergeEnvelope<'a, Counts: Serialize, Log: Serialize, Degraded: Serialize> {
    pub checks_summary: ChecksSummary,
    pub degraded_axes: &'a Degraded,
    pub has_blocking_findings: bool,
    pub merge_log: &'a Log,
    pub out: String,
    pub severity_counts: &'a Counts,
}

/// Stage tallies of a run summary; the counts sum to `total`.
/// Fields are declared in alphabetical order to match the wire output.
#[derive(Serialize)]
pub struct RunSummaryCounts {
    pub cancel: usize,
    pub done: usize,
    pub in_progress: usize,
    pub pending: usize,
    pub skipped: usize,
    pub total: usize,
}

/// One stage row of a run summary.
/// Fields are declared in alphabetical order to match the wire output.
#[derive(Serialize)]
pub struct RunSummaryStage<'a> {
    pub name: &'a str,
    pub status: String,
    pub tokens: Option<u64>,
}

/// Envelope printed by `aidp progress` and `aidp run state` in a JSON format.
/// Fields are declared in alphabetical order to match the wire output.
#[derive(Serialize)]
pub struct RunSummaryEnvelope<'a> {
    pub counts: RunSummaryCounts,
    pub pipeline: &'a str,
    pub run_dir: String,
    pub run_id: &'a str,
    pub stages: Vec<RunSummaryStage<'a>>,
    pub tokens: u64,
}
