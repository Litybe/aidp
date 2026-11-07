use super::run_loader::load_run;
use super::CommandOutput;
use crate::ctx::Ctx;
use crate::models::envelopes::{RunSummaryCounts, RunSummaryEnvelope, RunSummaryStage};
use aidp_core::run::RunState;
use aidp_core::run::StageStatus;
use aidp_core::Format;
use std::path::PathBuf;

pub struct ProgressArgs {
    pub run: String,
    pub workspace_root: Option<PathBuf>,
}

pub fn run(ctx: &Ctx, args: ProgressArgs) -> anyhow::Result<CommandOutput> {
    let workspace = ctx.cwd_or(args.workspace_root);
    let run_state = load_run(ctx, &workspace, &args.run)?;
    Ok(render_run_view(
        ctx.format(),
        &run_state,
        aidp_core::run::render_progress(&run_state),
    ))
}

/// The markdown progress of one run, shared by `aidp progress` and `aidp run state`.
pub fn render_run_progress(ctx: &Ctx, run: &str) -> anyhow::Result<CommandOutput> {
    let run_state = load_run(ctx, &ctx.workspace(), run)?;
    Ok(render_run_view(
        ctx.format(),
        &run_state,
        aidp_core::run::render_progress(&run_state),
    ))
}

/// Text when no format or `Text` is requested, the JSON value otherwise.
pub fn render_run_view(
    format: Option<Format>,
    run_state: &RunState,
    text: String,
) -> CommandOutput {
    match format {
        None | Some(Format::Text) => CommandOutput::text(text),
        Some(other) => CommandOutput::json(Some(other), run_summary_value(run_state)),
    }
}

/// Stage names, statuses and counters of a run as a JSON value.
/// Serializing the plain-data envelope cannot fail.
pub fn run_summary_value(run_state: &RunState) -> serde_json::Value {
    let count = |status: StageStatus| {
        run_state
            .stages
            .iter()
            .filter(|stage| stage.status == status)
            .count()
    };
    let envelope = RunSummaryEnvelope {
        counts: RunSummaryCounts {
            cancel: count(StageStatus::Cancel),
            done: count(StageStatus::Success),
            in_progress: count(StageStatus::InProgress),
            pending: count(StageStatus::Pending),
            skipped: count(StageStatus::Skipped),
            total: run_state.stages.len(),
        },
        pipeline: &run_state.pipeline_id,
        run_dir: run_state.run_dir.display().to_string(),
        run_id: &run_state.run_id,
        stages: run_state
            .stages
            .iter()
            .map(|stage| RunSummaryStage {
                name: &stage.name,
                status: stage.status.to_string(),
                tokens: stage.tokens_used,
            })
            .collect(),
        tokens: run_state
            .stages
            .iter()
            .filter_map(|stage| stage.tokens_used)
            .sum(),
    };
    serde_json::to_value(envelope).expect("run summary envelope is plain data")
}

#[cfg(test)]
#[path = "../../tests/unit/progress_tests.rs"]
mod tests;
