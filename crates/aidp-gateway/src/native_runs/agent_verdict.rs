//! The pure decision about the result a host posted for an agent job.

use super::reply::{read_reply, repair_instruction};
use crate::models::constants::agent_jobs::{
    REASON_REPLY_REJECTED, REASON_SESSION_FAILED, REASON_SESSION_RATE_LIMITED,
    REASON_SESSION_TIMED_OUT,
};
use crate::models::enums::agent_outcome::AgentOutcome;
use crate::models::enums::agent_verdict::AgentVerdict;
use aidp_core::models::enums::agent_role::AgentRole;
use std::path::Path;

/// Attempt number of the first job of a label; a repair job is attempt two.
pub const FIRST_ATTEMPT: u32 = 1;

fn failure(prefix: &str, reason: Option<&str>) -> AgentVerdict {
    AgentVerdict::Failed(format!("{prefix}: {}", reason.unwrap_or_default()))
}

/// Decides what a result means: the reply read from `reply_dir` is accepted, repaired once or refused.
pub fn judge(
    outcome: AgentOutcome,
    reason: Option<&str>,
    reply_dir: &Path,
    role: AgentRole,
    declared_outputs: &[String],
    attempt: u32,
) -> AgentVerdict {
    match outcome {
        AgentOutcome::Cancelled => AgentVerdict::Cancelled,
        AgentOutcome::Failed => failure(REASON_SESSION_FAILED, reason),
        AgentOutcome::Timeout => failure(REASON_SESSION_TIMED_OUT, reason),
        AgentOutcome::RateLimited => failure(REASON_SESSION_RATE_LIMITED, reason),
        AgentOutcome::Completed => match read_reply(reply_dir, role, declared_outputs) {
            Ok(envelope) => AgentVerdict::Accepted(envelope),
            Err(error) if attempt > FIRST_ATTEMPT => {
                AgentVerdict::Failed(format!("{REASON_REPLY_REJECTED}: {error}"))
            }
            Err(error) => AgentVerdict::Repair {
                error: error.to_string(),
                instruction: repair_instruction(&error),
            },
        },
    }
}

#[cfg(test)]
#[path = "../../tests/unit/agent_verdict_tests.rs"]
mod tests;
