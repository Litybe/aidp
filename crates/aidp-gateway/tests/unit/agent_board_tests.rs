use super::*;
use crate::models::agent_session_report::AgentSessionReport;
use crate::models::constants::agent_jobs::REASON_REPLY_REJECTED;
use crate::native_runs::agent_job_builder::fixture::{sample_job, RUN_ID, VALID_REPLY};
use crate::native_runs::reply::REPLY_FILE;

const LABEL: &str = "implement";
const OTHER_LABEL: &str = "implement#T2";
const NOW: i64 = 1_800_000_000_000;
const SECOND: i64 = 1000;
const OUTCOME_COMPLETED: &str = "completed";
const OUTCOME_FAILED: &str = "failed";
const OUTCOME_RATE_LIMITED: &str = "rate_limited";
const SESSION_ID: &str = "ada:3f2a";

fn open_board(dir: &Path) -> AgentBoard {
    let board = AgentBoard::default();
    board.ensure_open(dir).unwrap();
    board
}

fn publish(board: &AgentBoard, label: &str, dir: &Path) -> Published {
    board
        .publish_or_attach(label, AgentJobKind::Initial, sample_job(label, dir), NOW)
        .unwrap()
}

fn claim(board: &AgentBoard, job_id: &str, now: i64) -> AgentClaim {
    board
        .claim(job_id, &AgentClaimRequest::default(), now)
        .unwrap()
}

fn result(token: &str, outcome: &str, reason: Option<&str>) -> AgentJobResult {
    AgentJobResult {
        outcome: outcome.to_string(),
        reason: reason.map(str::to_string),
        actor: Some("ada".to_string()),
        agent: Some(AgentSessionReport {
            claim_token: token.to_string(),
            session_id: Some(SESSION_ID.to_string()),
            duration_ms: Some(10),
            ..AgentSessionReport::default()
        }),
    }
}

fn session_error(error: &anyhow::Error) -> &SessionError {
    error.downcast_ref::<SessionError>().unwrap()
}

fn write_reply(dir: &Path, text: &str) {
    std::fs::write(dir.join(REPLY_FILE), text).unwrap();
}

#[test]
fn publishing_numbers_the_jobs_and_writes_them_to_disk() {
    let dir = tempfile::tempdir().unwrap();
    let board = open_board(dir.path());
    let first = publish(&board, LABEL, dir.path());
    let second = publish(&board, OTHER_LABEL, dir.path());
    assert_eq!(first.job_id, format!("agent:{RUN_ID}:1"));
    assert_eq!(second.job_id, format!("agent:{RUN_ID}:2"));
    assert!(first.fresh && second.fresh);
    assert!(dir.path().join(AGENT_JOBS_DIR).join("1.json").is_file());
    assert!(dir.path().join(AGENT_JOBS_DIR).join("2.json").is_file());
}

#[test]
fn a_job_id_round_trips_through_its_parser() {
    let job_id = job_id_of(RUN_ID, 7);
    assert_eq!(parse_job_id(&job_id), Some((RUN_ID, 7)));
    assert_eq!(parse_job_id("action:step:1"), None);
    assert_eq!(parse_job_id("agent:run:x"), None);
}

#[test]
fn publishing_again_for_an_open_label_attaches_to_it() {
    let dir = tempfile::tempdir().unwrap();
    let board = open_board(dir.path());
    let first = publish(&board, LABEL, dir.path());
    let again = publish(&board, LABEL, dir.path());
    assert_eq!(again.job_id, first.job_id);
    assert!(!again.fresh);
}

#[test]
fn a_claim_hides_the_job_and_takes_a_lease() {
    let dir = tempfile::tempdir().unwrap();
    let board = open_board(dir.path());
    let published = publish(&board, LABEL, dir.path());
    let listed = board.summaries(NOW).unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, published.job_id);
    assert_eq!(listed[0].name, LABEL);
    assert!(!listed[0].agent.resumed);
    let claimed = claim(&board, &published.job_id, NOW);
    assert!(!claimed.claim_token.is_empty());
    assert_eq!(claimed.lease_seconds, LEASE_SECONDS);
    assert!(!claimed.resumed);
    assert_eq!(claimed.job.id, published.job_id);
    assert!(board.summaries(NOW).unwrap().is_empty());
    assert!(board.has_claimed());
}

#[test]
fn a_second_claim_with_a_live_lease_conflicts() {
    let dir = tempfile::tempdir().unwrap();
    let board = open_board(dir.path());
    let published = publish(&board, LABEL, dir.path());
    claim(&board, &published.job_id, NOW);
    let error = board
        .claim(
            &published.job_id,
            &AgentClaimRequest::default(),
            NOW + SECOND,
        )
        .unwrap_err();
    assert!(matches!(session_error(&error), SessionError::Conflict(_)));
}

#[test]
fn claiming_an_unknown_job_is_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let board = open_board(dir.path());
    let error = board
        .claim(&job_id_of(RUN_ID, 99), &AgentClaimRequest::default(), NOW)
        .unwrap_err();
    assert!(matches!(session_error(&error), SessionError::NotFound(_)));
}

#[test]
fn a_heartbeat_renews_the_lease_and_refuses_a_wrong_token() {
    let dir = tempfile::tempdir().unwrap();
    let board = open_board(dir.path());
    let published = publish(&board, LABEL, dir.path());
    let claimed = claim(&board, &published.job_id, NOW);
    let beat = board
        .heartbeat(&published.job_id, &claimed.claim_token, NOW + 30 * SECOND)
        .unwrap();
    assert!(!beat.cancel_requested);
    assert_ne!(beat.lease_expires_at, claimed.lease_expires_at);
    let error = board
        .heartbeat(&published.job_id, "other", NOW + 31 * SECOND)
        .unwrap_err();
    assert!(matches!(session_error(&error), SessionError::Conflict(_)));
}

#[test]
fn an_expired_lease_returns_the_job_resumed_with_the_resume_note() {
    let dir = tempfile::tempdir().unwrap();
    let board = open_board(dir.path());
    let published = publish(&board, LABEL, dir.path());
    let first = claim(&board, &published.job_id, NOW);
    let expiry = NOW + (LEASE_SECONDS as i64 + 1) * SECOND;
    assert!(board.sweep(expiry).unwrap());
    let listed = board.summaries(expiry).unwrap();
    assert_eq!(listed.len(), 1);
    assert!(listed[0].agent.resumed);
    let stale = board
        .heartbeat(&published.job_id, &first.claim_token, expiry)
        .unwrap_err();
    assert!(matches!(session_error(&stale), SessionError::Conflict(_)));
    let second = claim(&board, &published.job_id, expiry);
    assert!(second.resumed);
    assert_ne!(second.claim_token, first.claim_token);
    assert!(second.job.prompt.text.ends_with(RESUME_INSTRUCTION));
    assert_eq!(
        second.job.prompt.sha256,
        crate::native_runs::agent_job_builder::sha256_hex(&second.job.prompt.text)
    );
    assert_eq!(
        second.job.prompt.chars,
        second.job.prompt.text.chars().count()
    );
}

#[test]
fn a_second_expiry_fails_the_job() {
    let dir = tempfile::tempdir().unwrap();
    let board = open_board(dir.path());
    let published = publish(&board, LABEL, dir.path());
    claim(&board, &published.job_id, NOW);
    let lease = (LEASE_SECONDS as i64 + 1) * SECOND;
    board.sweep(NOW + lease).unwrap();
    claim(&board, &published.job_id, NOW + lease);
    board.sweep(NOW + 2 * lease).unwrap();
    let record = board.record(&published.job_id).unwrap();
    assert_eq!(record.state, AgentJobState::Done);
    assert_eq!(
        record.verdict,
        Some(AgentVerdict::Failed(REASON_INTERRUPTED_TWICE.to_string()))
    );
}

#[test]
fn a_job_nobody_claims_fails_after_the_claim_timeout() {
    let dir = tempfile::tempdir().unwrap();
    let board = open_board(dir.path());
    let published = publish(&board, LABEL, dir.path());
    assert!(!board.sweep(NOW + CLAIM_TIMEOUT_MS).unwrap());
    assert!(board.sweep(NOW + CLAIM_TIMEOUT_MS + 1).unwrap());
    let record = board.record(&published.job_id).unwrap();
    assert_eq!(
        record.verdict,
        Some(AgentVerdict::Failed(REASON_NO_HOST.to_string()))
    );
}

#[test]
fn a_session_that_outlives_its_timeout_fails_even_with_a_live_lease() {
    let dir = tempfile::tempdir().unwrap();
    let board = open_board(dir.path());
    let published = publish(&board, LABEL, dir.path());
    let claimed = claim(&board, &published.job_id, NOW);
    let limit = i64::from(claimed.job.timeout_minutes) * MILLIS_PER_MINUTE + SESSION_GRACE_MS;
    let step = (LEASE_SECONDS as i64 - 1) * SECOND;
    let mut beat_at = NOW;
    while beat_at + step < NOW + limit - SECOND {
        beat_at += step;
        board
            .heartbeat(&published.job_id, &claimed.claim_token, beat_at)
            .unwrap();
    }
    assert!(board.sweep(NOW + limit + 1).unwrap());
    let record = board.record(&published.job_id).unwrap();
    assert_eq!(
        record.verdict,
        Some(AgentVerdict::Failed(REASON_HOST_TIMEOUT.to_string()))
    );
}

#[test]
fn a_valid_reply_settles_the_job() {
    let dir = tempfile::tempdir().unwrap();
    write_reply(dir.path(), VALID_REPLY);
    let board = open_board(dir.path());
    let published = publish(&board, LABEL, dir.path());
    let claimed = claim(&board, &published.job_id, NOW);
    let ack = board
        .deliver(
            &published.job_id,
            &result(&claimed.claim_token, OUTCOME_COMPLETED, None),
            NOW,
        )
        .unwrap();
    assert!(ack.accepted && !ack.duplicate);
    assert_eq!(ack.next, NEXT_SETTLED);
    assert_eq!(ack.repair_job_id, None);
    let record = board.record(&published.job_id).unwrap();
    assert_eq!(record.state, AgentJobState::Done);
    assert!(matches!(record.verdict, Some(AgentVerdict::Accepted(_))));
    assert_eq!(
        record.report.and_then(|report| report.session_id),
        Some(SESSION_ID.to_string())
    );
}

#[test]
fn an_invalid_reply_publishes_one_repair_job() {
    let dir = tempfile::tempdir().unwrap();
    let board = open_board(dir.path());
    let published = publish(&board, LABEL, dir.path());
    let claimed = claim(&board, &published.job_id, NOW);
    let ack = board
        .deliver(
            &published.job_id,
            &result(&claimed.claim_token, OUTCOME_COMPLETED, None),
            NOW,
        )
        .unwrap();
    assert_eq!(ack.next, NEXT_REPAIR);
    let repair_id = ack.repair_job_id.clone().unwrap();
    assert_eq!(repair_id, job_id_of(RUN_ID, 2));
    let repair = board.record(&repair_id).unwrap();
    assert_eq!(repair.job.job_kind, AgentJobKind::Repair);
    assert_eq!(repair.job.attempt, 2);
    assert!(repair.job.prompt.text.contains(REPAIR_HEADING));
    assert_eq!(board.summaries(NOW).unwrap()[0].id, repair_id);
    let second = claim(&board, &repair_id, NOW);
    let failed = board
        .deliver(
            &repair_id,
            &result(&second.claim_token, OUTCOME_COMPLETED, None),
            NOW,
        )
        .unwrap();
    assert_eq!(failed.next, NEXT_SETTLED);
    assert!(failed.reason.unwrap().starts_with(REASON_REPLY_REJECTED));
}

#[test]
fn a_repair_job_accepts_the_fixed_reply() {
    let dir = tempfile::tempdir().unwrap();
    let board = open_board(dir.path());
    let published = publish(&board, LABEL, dir.path());
    let claimed = claim(&board, &published.job_id, NOW);
    let ack = board
        .deliver(
            &published.job_id,
            &result(&claimed.claim_token, OUTCOME_COMPLETED, None),
            NOW,
        )
        .unwrap();
    let repair_id = ack.repair_job_id.unwrap();
    write_reply(dir.path(), VALID_REPLY);
    let second = claim(&board, &repair_id, NOW);
    let settled = board
        .deliver(
            &repair_id,
            &result(&second.claim_token, OUTCOME_COMPLETED, None),
            NOW,
        )
        .unwrap();
    assert_eq!(settled.next, NEXT_SETTLED);
    assert!(matches!(
        board.record(&repair_id).unwrap().verdict,
        Some(AgentVerdict::Accepted(_))
    ));
}

#[test]
fn the_same_result_posted_twice_is_a_duplicate_and_another_one_conflicts() {
    let dir = tempfile::tempdir().unwrap();
    write_reply(dir.path(), VALID_REPLY);
    let board = open_board(dir.path());
    let published = publish(&board, LABEL, dir.path());
    let claimed = claim(&board, &published.job_id, NOW);
    let posted = result(&claimed.claim_token, OUTCOME_COMPLETED, None);
    board.deliver(&published.job_id, &posted, NOW).unwrap();
    let again = board.deliver(&published.job_id, &posted, NOW).unwrap();
    assert!(again.accepted && again.duplicate);
    let different = result(&claimed.claim_token, OUTCOME_FAILED, Some("late"));
    let error = board
        .deliver(&published.job_id, &different, NOW)
        .unwrap_err();
    assert!(matches!(session_error(&error), SessionError::Conflict(_)));
}

#[test]
fn a_result_with_a_wrong_token_conflicts() {
    let dir = tempfile::tempdir().unwrap();
    let board = open_board(dir.path());
    let published = publish(&board, LABEL, dir.path());
    claim(&board, &published.job_id, NOW);
    let error = board
        .deliver(
            &published.job_id,
            &result("other", OUTCOME_COMPLETED, None),
            NOW,
        )
        .unwrap_err();
    assert!(matches!(session_error(&error), SessionError::Conflict(_)));
}

#[test]
fn an_unknown_outcome_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let board = open_board(dir.path());
    let published = publish(&board, LABEL, dir.path());
    let claimed = claim(&board, &published.job_id, NOW);
    let error = board
        .deliver(
            &published.job_id,
            &result(&claimed.claim_token, "exploded", None),
            NOW,
        )
        .unwrap_err();
    assert!(matches!(session_error(&error), SessionError::Config(_)));
}

#[test]
fn a_rate_limit_carries_the_retry_delay_in_its_reason() {
    let dir = tempfile::tempdir().unwrap();
    let board = open_board(dir.path());
    let published = publish(&board, LABEL, dir.path());
    let claimed = claim(&board, &published.job_id, NOW);
    let mut posted = result(
        &claimed.claim_token,
        OUTCOME_RATE_LIMITED,
        Some("slow down"),
    );
    posted.agent.as_mut().unwrap().retry_after_seconds = Some(90);
    let ack = board.deliver(&published.job_id, &posted, NOW).unwrap();
    assert_eq!(
        ack.reason.as_deref(),
        Some("session rate limited: slow down (retry after 90s)")
    );
}

#[test]
fn cancelling_closes_open_jobs_and_ignores_late_results() {
    let dir = tempfile::tempdir().unwrap();
    let board = open_board(dir.path());
    let published = publish(&board, LABEL, dir.path());
    let claimed = claim(&board, &published.job_id, NOW);
    let pending = publish(&board, OTHER_LABEL, dir.path());
    assert_eq!(board.cancel_all().unwrap(), 2);
    let beat = board
        .heartbeat(&published.job_id, &claimed.claim_token, NOW + SECOND)
        .unwrap();
    assert!(beat.cancel_requested);
    let late = board
        .deliver(
            &published.job_id,
            &result(&claimed.claim_token, OUTCOME_COMPLETED, None),
            NOW,
        )
        .unwrap();
    assert!(!late.accepted);
    assert_eq!(late.next, NEXT_IGNORED);
    let error = board
        .claim(&pending.job_id, &AgentClaimRequest::default(), NOW)
        .unwrap_err();
    assert!(matches!(session_error(&error), SessionError::Conflict(_)));
    assert!(board.summaries(NOW).unwrap().is_empty());
}

#[test]
fn a_reloaded_board_keeps_its_jobs_and_never_reuses_an_id() {
    let dir = tempfile::tempdir().unwrap();
    let board = open_board(dir.path());
    let published = publish(&board, LABEL, dir.path());
    board.cancel_all().unwrap();
    let reloaded = open_board(dir.path());
    assert_eq!(
        reloaded.record(&published.job_id).unwrap().state,
        AgentJobState::Cancelled
    );
    let fresh = publish(&reloaded, LABEL, dir.path());
    assert!(fresh.fresh);
    assert_eq!(fresh.job_id, job_id_of(RUN_ID, 2));
}

#[test]
fn an_unconsumed_verdict_is_replayed_after_a_reload_until_it_is_consumed() {
    let dir = tempfile::tempdir().unwrap();
    write_reply(dir.path(), VALID_REPLY);
    let board = open_board(dir.path());
    let published = publish(&board, LABEL, dir.path());
    let claimed = claim(&board, &published.job_id, NOW);
    board
        .deliver(
            &published.job_id,
            &result(&claimed.claim_token, OUTCOME_COMPLETED, None),
            NOW,
        )
        .unwrap();
    let reloaded = open_board(dir.path());
    let replay = publish(&reloaded, LABEL, dir.path());
    assert_eq!(replay.job_id, published.job_id);
    assert!(!replay.fresh);
    reloaded.consume(&published.job_id).unwrap();
    let next = publish(&reloaded, LABEL, dir.path());
    assert!(next.fresh);
    assert_ne!(next.job_id, published.job_id);
}

#[test]
fn a_claimed_job_survives_a_reload_with_its_token() {
    let dir = tempfile::tempdir().unwrap();
    let board = open_board(dir.path());
    let published = publish(&board, LABEL, dir.path());
    let claimed = claim(&board, &published.job_id, NOW);
    let reloaded = open_board(dir.path());
    let attached = publish(&reloaded, LABEL, dir.path());
    assert_eq!(attached.job_id, published.job_id);
    assert!(reloaded
        .heartbeat(&published.job_id, &claimed.claim_token, NOW + SECOND)
        .is_ok());
}

#[test]
fn a_cancelled_job_ignores_a_result_only_for_the_token_holder() {
    let dir = tempfile::tempdir().unwrap();
    let board = open_board(dir.path());
    let published = publish(&board, LABEL, dir.path());
    let claimed = claim(&board, &published.job_id, NOW);
    board.cancel_all().unwrap();
    let error = board
        .deliver(
            &published.job_id,
            &result("other", OUTCOME_COMPLETED, None),
            NOW,
        )
        .unwrap_err();
    assert!(matches!(session_error(&error), SessionError::Conflict(_)));
    let ignored = board
        .deliver(
            &published.job_id,
            &result(&claimed.claim_token, OUTCOME_COMPLETED, None),
            NOW,
        )
        .unwrap();
    assert_eq!(ignored.next, NEXT_IGNORED);
}

#[test]
fn a_host_that_lost_its_lease_cannot_heartbeat_or_deliver_before_the_sweep() {
    let dir = tempfile::tempdir().unwrap();
    let board = open_board(dir.path());
    let published = publish(&board, LABEL, dir.path());
    let claimed = claim(&board, &published.job_id, NOW);
    write_reply(dir.path(), VALID_REPLY);
    let late = NOW + (LEASE_SECONDS as i64 + 1) * SECOND;
    let delivered = board
        .deliver(
            &published.job_id,
            &result(&claimed.claim_token, OUTCOME_COMPLETED, None),
            late,
        )
        .unwrap_err();
    assert!(matches!(
        session_error(&delivered),
        SessionError::Conflict(_)
    ));
    let beat = board
        .heartbeat(&published.job_id, &claimed.claim_token, late)
        .unwrap_err();
    assert!(matches!(session_error(&beat), SessionError::Conflict(_)));
    let record = board.record(&published.job_id).unwrap();
    assert_eq!(record.state, AgentJobState::Pending);
    assert!(record.resumed);
}

#[test]
fn a_resumed_claim_does_not_repeat_a_resume_note_the_prompt_already_carries() {
    let dir = tempfile::tempdir().unwrap();
    let board = open_board(dir.path());
    let mut template = sample_job(LABEL, dir.path());
    template.prompt = prompt_of(&format!(
        "{}{RESUME_HEADING}{RESUME_INSTRUCTION}",
        template.prompt.text
    ));
    let published = board
        .publish_or_attach(LABEL, AgentJobKind::Initial, template, NOW)
        .unwrap();
    claim(&board, &published.job_id, NOW);
    let lease = (LEASE_SECONDS as i64 + 1) * SECOND;
    board.sweep(NOW + lease).unwrap();
    let second = claim(&board, &published.job_id, NOW + lease);
    assert!(second.resumed);
    assert_eq!(second.job.prompt.text.matches(RESUME_HEADING).count(), 1);
}
