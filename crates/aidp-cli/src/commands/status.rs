use super::CommandOutput;
use crate::ctx::Ctx;
use std::path::{Path, PathBuf};

use aidp_core::run::{INPUT_JIRA_KEY, INPUT_TARGET, RUN_STATE_FILE};

pub(super) const PIPELINES_DIR: &str = "pipelines";
pub(super) const AIDP_DIR: &str = ".aidp";
const RUN_SCAN_DEPTH: usize = 2;

pub struct StatusArgs {
    pub format: Option<aidp_core::Format>,
    pub workspace_root: Option<PathBuf>,
}

#[derive(serde::Serialize)]
struct RunSummary {
    run_id: String,
    pipeline_id: String,
    target: Option<String>,
    overall_status: String,
    current_stage: Option<String>,
    stages_total: usize,
    stages_done: usize,
    started_at: String,
}

pub fn run(ctx: &Ctx, args: StatusArgs) -> anyhow::Result<CommandOutput> {
    let workspace = ctx.cwd_or(args.workspace_root);
    let mut runs = collect_runs(&workspace);
    runs.sort_by(|a, b| b.started_at.cmp(&a.started_at));
    runs.dedup_by(|a, b| a.run_id == b.run_id);
    let rendered = crate::output::render_value(
        args.format.unwrap_or_default(),
        &serde_json::to_value(&runs)?,
    )?;
    Ok(CommandOutput::text(rendered))
}

/// The roots that hold runs: the vault's folder for this repo, and the legacy
/// `.aidp/pipelines/` holding pre-v7 runs.
fn run_roots(workspace: &Path) -> Vec<(PathBuf, usize)> {
    let mut roots: Vec<(PathBuf, usize)> = Vec::new();
    if let Some(vault_root) = aidp_core::vault::resolve(None, workspace) {
        roots.push((
            vault_root.repo_dir(&aidp_core::vault::repo_segment(workspace)),
            RUN_SCAN_DEPTH,
        ));
    }
    roots.push((workspace.join(AIDP_DIR).join(PIPELINES_DIR), RUN_SCAN_DEPTH));
    roots
}

/// Every run found under the roots; a directory is a run only if it holds a
/// `run-state.json`.
fn collect_runs(workspace: &Path) -> Vec<RunSummary> {
    let mut runs: Vec<RunSummary> = Vec::new();
    for (pipelines_dir, max_depth) in run_roots(workspace) {
        if !pipelines_dir.exists() {
            continue;
        }
        for entry in walkdir::WalkDir::new(&pipelines_dir)
            .max_depth(max_depth)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if !entry.file_type().is_dir() || entry.path() == pipelines_dir {
                continue;
            }
            let Some(run_id) = entry.file_name().to_str().map(str::to_string) else {
                continue;
            };
            if let Some(summary) = summarize_run(run_id, &entry.path().join(RUN_STATE_FILE)) {
                runs.push(summary);
            }
        }
    }
    runs
}

fn summarize_run(run_id: String, state_file: &Path) -> Option<RunSummary> {
    if !state_file.exists() {
        return None;
    }
    let run_state: aidp_core::run::RunState = std::fs::read_to_string(state_file)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())?;
    let stages_done = run_state
        .stages
        .iter()
        .filter(|s| {
            matches!(
                s.status,
                aidp_core::run::StageStatus::Success
                    | aidp_core::run::StageStatus::Cancel
                    | aidp_core::run::StageStatus::Skipped
            )
        })
        .count();
    let current_stage = run_state
        .stages
        .iter()
        .find(|s| matches!(s.status, aidp_core::run::StageStatus::InProgress))
        .map(|s| s.name.clone())
        .or_else(|| {
            run_state
                .stages
                .iter()
                .find(|s| matches!(s.status, aidp_core::run::StageStatus::Pending))
                .map(|s| s.name.clone())
        });
    let overall = aidp_core::run::overall_status(&run_state);
    let target = run_state
        .inputs
        .get(INPUT_TARGET)
        .or_else(|| run_state.inputs.get(INPUT_JIRA_KEY))
        .and_then(|value| value.as_str())
        .map(|s| s.to_string());
    Some(RunSummary {
        run_id,
        pipeline_id: run_state.pipeline_id,
        target,
        overall_status: overall.into(),
        current_stage,
        stages_total: run_state.stages.len(),
        stages_done,
        started_at: run_state.started_at,
    })
}
