//! A stage session run by a host: publish the job, park until a verdict exists, follow one repair job.

use super::agent_board::AgentBoard;
use super::dispatch::{add_usage, priced_usage};
use super::state::RunControl;
use crate::models::agent_job::AgentJob;
use crate::models::agent_usage_report::AgentUsageReport;
use crate::models::constants::agent_jobs::{REASON_STALE_JOB, WAKE_SLICE};
use crate::models::enums::agent_job_kind::AgentJobKind;
use crate::models::enums::agent_job_state::AgentJobState;
use crate::models::enums::agent_verdict::AgentVerdict;
use crate::models::enums::session_outcome::SessionOutcome;
use crate::models::session_run::SessionRun;
use aidp_core::run::StageUsage;
use aidp_core::tokens::TokenUsage;
use std::path::Path;

/// What `external_attempts` needs to know about the session it waits for.
pub struct ExternalSpec<'a> {
    pub run_dir: &'a Path,
    pub label: &'a str,
    /// The initial job; the board numbers it, or attaches to the open job of the label.
    pub template: AgentJob,
}

/// The usage a host reported, priced by the tier of the model it resolved.
pub fn usage_of_report(report: &AgentUsageReport, model: Option<&str>) -> StageUsage {
    let counted = TokenUsage {
        input_tokens: report.input_tokens,
        output_tokens: report.output_tokens,
        cache_creation_input_tokens: report.cache_write_tokens,
        cache_read_input_tokens: report.cache_read_tokens,
        ..TokenUsage::default()
    };
    priced_usage(&counted, model, report.cost_usd)
}

fn finish(outcome: SessionOutcome, usage: Option<StageUsage>) -> SessionRun {
    SessionRun {
        outcome,
        usage,
        duration_ms: 0,
    }
}

/// Runs the label as agent jobs: at most an initial job and one repair job, waiting on the host.
pub fn external_attempts(
    board: &AgentBoard,
    control: &RunControl,
    attach_session: &dyn Fn(&str) -> anyhow::Result<()>,
    spec: ExternalSpec<'_>,
) -> anyhow::Result<SessionRun> {
    board.ensure_open(spec.run_dir)?;
    let reply_file = spec.template.io.reply_file.clone();
    let published = board.publish_or_attach(
        spec.label,
        AgentJobKind::Initial,
        spec.template,
        crate::time::now_ms(),
    )?;
    if published.fresh {
        let _ = std::fs::remove_file(&reply_file);
    }
    let mut job_id = published.job_id;
    let mut usage: Option<StageUsage> = None;
    loop {
        if control.is_cancelled() {
            board.cancel_all()?;
            return Ok(finish(SessionOutcome::Cancelled, usage));
        }
        board.sweep(crate::time::now_ms())?;
        let Some(record) = board.record(&job_id) else {
            return Ok(finish(
                SessionOutcome::Failed(REASON_STALE_JOB.to_string()),
                usage,
            ));
        };
        match record.state {
            AgentJobState::Pending | AgentJobState::Claimed => {
                control.wait_for_wake(WAKE_SLICE);
                continue;
            }
            AgentJobState::Cancelled => {
                return Ok(finish(SessionOutcome::Cancelled, usage));
            }
            AgentJobState::Done => {}
        }
        if let Some(report) = record.report.as_ref() {
            let measured = report
                .usage
                .as_ref()
                .map(|reported| usage_of_report(reported, report.resolved_model.as_deref()));
            usage = add_usage(usage.take(), measured);
            if let Some(session_id) = report.session_id.as_deref() {
                attach_session(session_id)?;
            }
        }
        match record.verdict {
            Some(AgentVerdict::Accepted(envelope)) => {
                board.consume(&job_id)?;
                return Ok(finish(SessionOutcome::Completed(envelope), usage));
            }
            Some(AgentVerdict::Repair { .. }) => match record.repair_job_id {
                Some(next) => job_id = next,
                None => {
                    board.consume(&job_id)?;
                    return Ok(finish(
                        SessionOutcome::Failed(REASON_STALE_JOB.to_string()),
                        usage,
                    ));
                }
            },
            Some(AgentVerdict::Failed(reason)) => {
                board.consume(&job_id)?;
                return Ok(finish(SessionOutcome::Failed(reason), usage));
            }
            Some(AgentVerdict::Cancelled) | None => {
                board.consume(&job_id)?;
                return Ok(finish(SessionOutcome::Cancelled, usage));
            }
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/external_session_tests.rs"]
mod tests;
