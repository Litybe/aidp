use aidp_core::run::{render_progress, render_summary, RunState, StageState, StageStatus};
use std::path::PathBuf;

const RUN_ID: &str = "20260101-000000-abcd";
const PIPELINE_ID: &str = "render-snapshots";
const WORKFLOW_STATUS_DONE: &str = "done";
const STARTED_AT: &str = "2026-01-01T00:00:00+00:00";
const FINISHED_AT: &str = "2026-01-01T00:05:00+00:00";
const RELATIVE_RUN_DIR: &str = "run";
const SNAPSHOT_PROGRESS: &str = "render_progress";
const SNAPSHOT_SUMMARY: &str = "render_summary";

fn stage(name: &str, status: StageStatus, duration_ms: u64, tokens: u64) -> StageState {
    StageState {
        name: name.to_string(),
        status,
        started_at: Some(STARTED_AT.to_string()),
        finished_at: Some(FINISHED_AT.to_string()),
        duration_ms: Some(duration_ms),
        tokens_used: Some(tokens),
        ..Default::default()
    }
}

fn deterministic_state() -> RunState {
    RunState {
        run_id: RUN_ID.to_string(),
        pipeline_id: PIPELINE_ID.to_string(),
        run_dir: PathBuf::from(RELATIVE_RUN_DIR),
        started_at: STARTED_AT.to_string(),
        last_workflow_status: Some(WORKFLOW_STATUS_DONE.to_string()),
        stages: vec![
            stage("context", StageStatus::Success, 3_400, 1_200),
            stage("plan", StageStatus::Success, 62_000, 18_500),
            stage("implement", StageStatus::Cancel, 190_000, 1_250_000),
            stage("review", StageStatus::Skipped, 0, 0),
        ],
        ..Default::default()
    }
}

/// The progress table for a deterministic run stays stable.
#[test]
fn render_progress_matches_the_snapshot() {
    insta::assert_snapshot!(SNAPSHOT_PROGRESS, render_progress(&deterministic_state()));
}

/// The summary for a deterministic run stays stable.
#[test]
fn render_summary_matches_the_snapshot() {
    insta::assert_snapshot!(SNAPSHOT_SUMMARY, render_summary(&deterministic_state()));
}
