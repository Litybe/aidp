use super::*;
use crate::native_runs::agent_job_builder::fixture::VALID_REPLY;
use crate::native_runs::reply::REPLY_FILE;

const BROKEN_REPLY: &str = "not json";
const BOOM: &str = "boom";
const SECOND_ATTEMPT: u32 = 2;

fn dir_with_reply(reply: Option<&str>) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    if let Some(reply) = reply {
        std::fs::write(dir.path().join(REPLY_FILE), reply).unwrap();
    }
    dir
}

fn verdict(outcome: AgentOutcome, dir: &Path, attempt: u32) -> AgentVerdict {
    judge(outcome, Some(BOOM), dir, AgentRole::Executor, &[], attempt)
}

#[test]
fn a_valid_reply_is_accepted() {
    let dir = dir_with_reply(Some(VALID_REPLY));
    let verdict = verdict(AgentOutcome::Completed, dir.path(), FIRST_ATTEMPT);
    assert!(matches!(verdict, AgentVerdict::Accepted(envelope) if envelope.summary == "done"));
}

#[test]
fn an_invalid_reply_on_the_first_attempt_asks_for_a_repair() {
    let dir = dir_with_reply(Some(BROKEN_REPLY));
    let verdict = verdict(AgentOutcome::Completed, dir.path(), FIRST_ATTEMPT);
    let AgentVerdict::Repair { error, instruction } = verdict else {
        panic!("expected a repair verdict");
    };
    assert!(error.contains("JSON"));
    assert!(instruction.contains(&error));
}

#[test]
fn a_missing_reply_on_the_first_attempt_asks_for_a_repair() {
    let dir = dir_with_reply(None);
    let verdict = verdict(AgentOutcome::Completed, dir.path(), FIRST_ATTEMPT);
    assert!(matches!(verdict, AgentVerdict::Repair { .. }));
}

#[test]
fn an_invalid_reply_after_the_repair_fails_the_job() {
    let dir = dir_with_reply(Some(BROKEN_REPLY));
    let verdict = verdict(AgentOutcome::Completed, dir.path(), SECOND_ATTEMPT);
    let AgentVerdict::Failed(reason) = verdict else {
        panic!("expected a failed verdict");
    };
    assert!(reason.starts_with(&format!("{REASON_REPLY_REJECTED}: ")));
}

#[test]
fn a_valid_reply_after_the_repair_is_accepted() {
    let dir = dir_with_reply(Some(VALID_REPLY));
    let verdict = verdict(AgentOutcome::Completed, dir.path(), SECOND_ATTEMPT);
    assert!(matches!(verdict, AgentVerdict::Accepted(_)));
}

#[test]
fn a_failed_session_fails_without_a_repair() {
    let dir = dir_with_reply(Some(VALID_REPLY));
    assert_eq!(
        verdict(AgentOutcome::Failed, dir.path(), FIRST_ATTEMPT),
        AgentVerdict::Failed("session failed: boom".to_string())
    );
}

#[test]
fn a_timeout_fails_with_its_reason() {
    let dir = dir_with_reply(None);
    assert_eq!(
        verdict(AgentOutcome::Timeout, dir.path(), FIRST_ATTEMPT),
        AgentVerdict::Failed("session timed out: boom".to_string())
    );
}

#[test]
fn a_rate_limit_fails_with_its_reason() {
    let dir = dir_with_reply(None);
    assert_eq!(
        verdict(AgentOutcome::RateLimited, dir.path(), FIRST_ATTEMPT),
        AgentVerdict::Failed("session rate limited: boom".to_string())
    );
}

#[test]
fn a_cancelled_session_is_cancelled() {
    let dir = dir_with_reply(Some(VALID_REPLY));
    assert_eq!(
        verdict(AgentOutcome::Cancelled, dir.path(), FIRST_ATTEMPT),
        AgentVerdict::Cancelled
    );
}

#[test]
fn a_missing_reason_leaves_the_prefix_alone() {
    let dir = dir_with_reply(None);
    let verdict = judge(
        AgentOutcome::Failed,
        None,
        dir.path(),
        AgentRole::Executor,
        &[],
        FIRST_ATTEMPT,
    );
    assert_eq!(
        verdict,
        AgentVerdict::Failed(format!("{REASON_SESSION_FAILED}: "))
    );
}
