use super::*;
use crate::models::agent_claim_request::AgentClaimRequest;
use crate::models::agent_job_result::AgentJobResult;
use crate::models::agent_session_report::AgentSessionReport;
use crate::models::constants::agent_jobs::AGENT_JOBS_DIR;
use crate::native_runs::agent_job_builder::fixture::{sample_job, VALID_REPLY};
use crate::native_runs::reply::REPLY_FILE;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const LABEL: &str = "implement";
const HOST_WAIT: Duration = Duration::from_secs(10);
const HOST_STEP: Duration = Duration::from_millis(10);
const OUTCOME_COMPLETED: &str = "completed";
const OUTCOME_FAILED: &str = "failed";

/// What the fake host does with one job it claims.
struct Step {
    reply: Option<&'static str>,
    outcome: &'static str,
    reason: Option<&'static str>,
    session: &'static str,
}

fn completed(reply: Option<&'static str>, session: &'static str) -> Step {
    Step {
        reply,
        outcome: OUTCOME_COMPLETED,
        reason: None,
        session,
    }
}

fn report(token: &str, session: &str) -> AgentSessionReport {
    AgentSessionReport {
        claim_token: token.to_string(),
        session_id: Some(session.to_string()),
        usage: Some(AgentUsageReport {
            input_tokens: 10,
            output_tokens: 5,
            cost_usd: Some(0.5),
            complete: true,
            ..AgentUsageReport::default()
        }),
        ..AgentSessionReport::default()
    }
}

fn wait_for_pending(board: &AgentBoard) -> String {
    let started = Instant::now();
    loop {
        let pending = board.summaries(crate::time::now_ms()).unwrap();
        if let Some(first) = pending.first() {
            return first.id.clone();
        }
        assert!(started.elapsed() < HOST_WAIT, "no job was published");
        std::thread::sleep(HOST_STEP);
    }
}

fn play(
    board: &AgentBoard,
    control: &RunControl,
    dir: &Path,
    steps: &[Step],
    finished: &AtomicBool,
) {
    for step in steps {
        let job_id = wait_for_pending(board);
        let claim = board
            .claim(
                &job_id,
                &AgentClaimRequest::default(),
                crate::time::now_ms(),
            )
            .unwrap();
        board
            .heartbeat(&job_id, &claim.claim_token, crate::time::now_ms())
            .unwrap();
        if let Some(reply) = step.reply {
            std::fs::write(dir.join(REPLY_FILE), reply).unwrap();
        }
        let result = AgentJobResult {
            outcome: step.outcome.to_string(),
            reason: step.reason.map(str::to_string),
            actor: None,
            agent: Some(report(&claim.claim_token, step.session)),
        };
        board
            .deliver(&job_id, &result, crate::time::now_ms())
            .unwrap();
        control.wake();
    }
    while !finished.load(Ordering::SeqCst) {
        control.wake();
        std::thread::sleep(HOST_STEP);
    }
}

struct Played {
    run: SessionRun,
    sessions: Vec<String>,
}

fn run_with_host(dir: &Path, board: &AgentBoard, control: &RunControl, steps: &[Step]) -> Played {
    let sessions = Mutex::new(Vec::new());
    let finished = AtomicBool::new(false);
    let run = std::thread::scope(|scope| {
        let host = scope.spawn(|| play(board, control, dir, steps, &finished));
        let attach = |session: &str| {
            sessions.lock().unwrap().push(session.to_string());
            Ok(())
        };
        let outcome = external_attempts(
            board,
            control,
            &attach,
            ExternalSpec {
                run_dir: dir,
                label: LABEL,
                template: sample_job(LABEL, dir),
            },
        );
        finished.store(true, Ordering::SeqCst);
        host.join().unwrap();
        outcome.unwrap()
    });
    Played {
        run,
        sessions: sessions.into_inner().unwrap(),
    }
}

#[test]
fn a_valid_reply_completes_the_session_with_its_usage() {
    let dir = tempfile::tempdir().unwrap();
    let board = AgentBoard::default();
    let control = RunControl::default();
    let played = run_with_host(
        dir.path(),
        &board,
        &control,
        &[completed(Some(VALID_REPLY), "ada:one")],
    );
    assert!(matches!(
        played.run.outcome,
        SessionOutcome::Completed(ref envelope) if envelope.summary == "done"
    ));
    let usage = played.run.usage.unwrap();
    assert_eq!((usage.input, usage.output), (10, 5));
    assert_eq!(usage.cost_usd, Some(0.5));
    assert_eq!(played.sessions, vec!["ada:one".to_string()]);
}

#[test]
fn an_invalid_reply_is_repaired_by_a_second_job_and_the_usage_adds_up() {
    let dir = tempfile::tempdir().unwrap();
    let board = AgentBoard::default();
    let control = RunControl::default();
    let played = run_with_host(
        dir.path(),
        &board,
        &control,
        &[
            completed(None, "ada:one"),
            completed(Some(VALID_REPLY), "ada:two"),
        ],
    );
    assert!(matches!(played.run.outcome, SessionOutcome::Completed(_)));
    let usage = played.run.usage.unwrap();
    assert_eq!((usage.input, usage.output), (20, 10));
    assert_eq!(usage.cost_usd, Some(1.0));
    assert_eq!(
        played.sessions,
        vec!["ada:one".to_string(), "ada:two".to_string()]
    );
    assert!(dir.path().join(AGENT_JOBS_DIR).join("2.json").is_file());
    assert!(!dir.path().join(AGENT_JOBS_DIR).join("3.json").exists());
}

#[test]
fn a_reply_still_invalid_after_the_repair_fails_the_session() {
    let dir = tempfile::tempdir().unwrap();
    let board = AgentBoard::default();
    let control = RunControl::default();
    let played = run_with_host(
        dir.path(),
        &board,
        &control,
        &[completed(None, "ada:one"), completed(None, "ada:two")],
    );
    assert!(matches!(
        played.run.outcome,
        SessionOutcome::Failed(ref reason) if reason.starts_with("reply rejected after one repair")
    ));
}

#[test]
fn a_failed_session_fails_with_the_host_reason() {
    let dir = tempfile::tempdir().unwrap();
    let board = AgentBoard::default();
    let control = RunControl::default();
    let played = run_with_host(
        dir.path(),
        &board,
        &control,
        &[Step {
            reply: None,
            outcome: OUTCOME_FAILED,
            reason: Some("boom"),
            session: "ada:one",
        }],
    );
    assert!(matches!(
        played.run.outcome,
        SessionOutcome::Failed(ref reason) if reason == "session failed: boom"
    ));
}

#[test]
fn a_cancelled_run_ends_the_wait_and_closes_the_job() {
    let dir = tempfile::tempdir().unwrap();
    let board = AgentBoard::default();
    let control = RunControl::default();
    let attach = |_: &str| Ok(());
    let run = std::thread::scope(|scope| {
        let canceller = scope.spawn(|| {
            let job_id = wait_for_pending(&board);
            control.cancel();
            job_id
        });
        let run = external_attempts(
            &board,
            &control,
            &attach,
            ExternalSpec {
                run_dir: dir.path(),
                label: LABEL,
                template: sample_job(LABEL, dir.path()),
            },
        )
        .unwrap();
        let job_id = canceller.join().unwrap();
        assert_eq!(
            board.record(&job_id).unwrap().state,
            AgentJobState::Cancelled
        );
        run
    });
    assert!(matches!(run.outcome, SessionOutcome::Cancelled));
}

#[test]
fn a_restarted_run_attaches_to_the_claimed_job_instead_of_publishing_another() {
    let dir = tempfile::tempdir().unwrap();
    let before = AgentBoard::default();
    before.ensure_open(dir.path()).unwrap();
    let published = before
        .publish_or_attach(
            LABEL,
            AgentJobKind::Initial,
            sample_job(LABEL, dir.path()),
            crate::time::now_ms(),
        )
        .unwrap();
    let claim = before
        .claim(
            &published.job_id,
            &AgentClaimRequest::default(),
            crate::time::now_ms(),
        )
        .unwrap();
    std::fs::write(dir.path().join(REPLY_FILE), VALID_REPLY).unwrap();
    let after = AgentBoard::default();
    let control = RunControl::default();
    let finished = AtomicBool::new(false);
    let attach = |_: &str| Ok(());
    let run = std::thread::scope(|scope| {
        let host = scope.spawn(|| {
            let started = Instant::now();
            let result = AgentJobResult {
                outcome: OUTCOME_COMPLETED.to_string(),
                reason: None,
                actor: None,
                agent: Some(report(&claim.claim_token, "ada:one")),
            };
            loop {
                if after.ensure_open(dir.path()).is_ok()
                    && after
                        .deliver(&published.job_id, &result, crate::time::now_ms())
                        .is_ok()
                {
                    break;
                }
                assert!(started.elapsed() < HOST_WAIT);
                std::thread::sleep(HOST_STEP);
            }
            while !finished.load(Ordering::SeqCst) {
                control.wake();
                std::thread::sleep(HOST_STEP);
            }
        });
        let run = external_attempts(
            &after,
            &control,
            &attach,
            ExternalSpec {
                run_dir: dir.path(),
                label: LABEL,
                template: sample_job(LABEL, dir.path()),
            },
        )
        .unwrap();
        finished.store(true, Ordering::SeqCst);
        host.join().unwrap();
        run
    });
    assert!(matches!(run.outcome, SessionOutcome::Completed(_)));
    assert!(!dir.path().join(AGENT_JOBS_DIR).join("2.json").exists());
    assert!(dir.path().join(REPLY_FILE).is_file());
}

#[test]
fn a_fresh_job_removes_the_stale_reply_of_an_earlier_attempt() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(REPLY_FILE), VALID_REPLY).unwrap();
    let board = AgentBoard::default();
    let control = RunControl::default();
    let attach = |_: &str| Ok(());
    std::thread::scope(|scope| {
        let canceller = scope.spawn(|| {
            wait_for_pending(&board);
            control.cancel();
        });
        external_attempts(
            &board,
            &control,
            &attach,
            ExternalSpec {
                run_dir: dir.path(),
                label: LABEL,
                template: sample_job(LABEL, dir.path()),
            },
        )
        .unwrap();
        canceller.join().unwrap();
    });
    assert!(!dir.path().join(REPLY_FILE).exists());
}
