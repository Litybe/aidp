//! Handler decisions: foreign comments, lines outside the diff and own-PR verdicts never reach GitHub.

use super::*;
use crate::bus::Bus;
use crate::models::enums::pr_refusal_reason::PrRefusalReason;
use crate::pr_drafts::PrDrafts;
use crate::pr_viewed::PrViewed;
use crate::prs::PrStatusCache;
use crate::squad_prs::{SquadFetcher, SquadPrsCache};
use aidp_core::clock::FixedClock;
use aidp_core::models::process::{ProcessSpec, ScriptedOutput};
use aidp_core::process_runner::{ProcessRunner, ScriptedProcessRunner, SpawnedChild};
use aidp_integrations::models::squad_prs::SquadPrs;
use std::io;
use std::process::Output;
use std::sync::Mutex;

const REPO: &str = "acme/svc";
const NUMBER: u64 = 7;
const COMMENT_ID: u64 = 42;
const NOW: &str = "2026-01-01T00:00:00Z";
const HEAD_SHA: &str = "abc123";
const HEAD_REF: &str = "feat/x";
const ME: &str = "alice";
const OTHER: &str = "bob";
const FILE_PATH: &str = "src/a.rs";
const WRITE_METHOD_FLAG: &str = "-X";
const COMMENT_AUTHOR_BODY: &str = r#"{"user":{"login":"bob"}}"#;
const FILES_BODY: &str = r#"[{"filename":"src/a.rs","status":"modified","additions":1,"deletions":0,"patch":"@@ -1,2 +1,3 @@\n a\n+b\n c\n","sha":"blob1"}]"#;
const REVIEW_REPLY: &str = r#"{"id":9,"state":"COMMENTED","html_url":"u"}"#;
const DRAFT_ID: &str = "d1";
const DRAFT_BODY: &str = "nit";
const VALID_LINE: u32 = 2;
const OUTSIDE_LINE: u32 = 99;
const FAILED_STATUS: i32 = 1;

struct RecordingRunner {
    inner: ScriptedProcessRunner,
    specs: Mutex<Vec<ProcessSpec>>,
}

impl RecordingRunner {
    fn replying(stdout: &str) -> Self {
        let output = ScriptedOutput {
            status_code: 0,
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
        };
        RecordingRunner {
            inner: ScriptedProcessRunner::new().with_output(output),
            specs: Mutex::new(Vec::new()),
        }
    }

    fn failing() -> Self {
        let output = ScriptedOutput {
            status_code: FAILED_STATUS,
            stdout: Vec::new(),
            stderr: b"boom".to_vec(),
        };
        RecordingRunner {
            inner: ScriptedProcessRunner::new().with_output(output),
            specs: Mutex::new(Vec::new()),
        }
    }

    fn call_count(&self) -> usize {
        self.specs.lock().unwrap().len()
    }

    fn wrote(&self) -> bool {
        self.specs
            .lock()
            .unwrap()
            .iter()
            .any(|spec| spec.args.iter().any(|arg| arg == WRITE_METHOD_FLAG))
    }
}

impl ProcessRunner for RecordingRunner {
    fn output(&self, spec: &ProcessSpec) -> io::Result<Output> {
        self.specs.lock().unwrap().push(spec.clone());
        self.inner.output(spec)
    }

    fn spawn(&self, spec: &ProcessSpec) -> io::Result<Box<dyn SpawnedChild>> {
        self.inner.spawn(spec)
    }
}

struct NoFetch;

impl SquadFetcher for NoFetch {
    fn fetch(&self, _previous: Option<&SquadPrs>) -> Result<SquadPrs, String> {
        Err(String::new())
    }
}

struct Fixture {
    cache: PrStatusCache,
    bus: Bus,
    squad: SquadPrsCache,
    drafts: PrDrafts,
    viewed: PrViewed,
    _dir: tempfile::TempDir,
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    Fixture {
        cache: PrStatusCache::new(Box::new(FixedClock::from_rfc3339(NOW))),
        bus: Bus::new(),
        squad: SquadPrsCache::new(Box::new(FixedClock::from_rfc3339(NOW)), Box::new(NoFetch)),
        drafts: PrDrafts::new(dir.path()),
        viewed: PrViewed::new(dir.path()),
        _dir: dir,
    }
}

fn context<'a>(fixture: &'a Fixture, runner: &'a dyn ProcessRunner) -> PrDetailContext<'a> {
    PrDetailContext {
        cache: &fixture.cache,
        bus: &fixture.bus,
        squad: &fixture.squad,
        drafts: &fixture.drafts,
        viewed: &fixture.viewed,
        runner,
        workspace_root: None,
    }
}

fn state(author: &str) -> PrWriteState {
    PrWriteState {
        head_sha: HEAD_SHA.to_string(),
        head_ref: HEAD_REF.to_string(),
        author: Some(author.to_string()),
        viewer: Some(ME.to_string()),
    }
}

fn draft(line: u32) -> ReviewDraft {
    ReviewDraft {
        id: DRAFT_ID.to_string(),
        path: FILE_PATH.to_string(),
        line,
        side: DiffSide::Right,
        start_line: None,
        start_side: None,
        body: DRAFT_BODY.to_string(),
    }
}

fn comment_request() -> PrCommentRequest {
    PrCommentRequest {
        repo: REPO.to_string(),
        number: NUMBER,
        body: DRAFT_BODY.to_string(),
    }
}

fn review_request(event: ReviewEvent) -> PrReviewRequest {
    PrReviewRequest {
        repo: REPO.to_string(),
        number: NUMBER,
        event,
        body: String::new(),
    }
}

fn status_of(result: Result<Value, PrRouteError>) -> u16 {
    result.expect_err("an error reply").status()
}

#[test]
fn editing_a_foreign_comment_is_forbidden_without_a_write() {
    let fixture = fixture();
    let runner = RecordingRunner::replying(COMMENT_AUTHOR_BODY);
    let ctx = context(&fixture, &runner);
    let result = comment_edit_with_state(
        &ctx,
        &state(OTHER),
        CommentKind::Conversation,
        COMMENT_ID,
        comment_request(),
    );
    assert_eq!(status_of(result), ApiStatus::Forbidden.status());
    assert!(!runner.wrote());
}

#[test]
fn deleting_a_foreign_comment_is_forbidden_without_a_write() {
    let fixture = fixture();
    let runner = RecordingRunner::replying(COMMENT_AUTHOR_BODY);
    let ctx = context(&fixture, &runner);
    let query = PrDetailQuery {
        repo: REPO.to_string(),
        number: NUMBER,
        path: None,
        page: None,
    };
    let result =
        comment_delete_with_state(&ctx, &state(OTHER), CommentKind::Review, COMMENT_ID, &query);
    assert_eq!(status_of(result), ApiStatus::Forbidden.status());
    assert!(!runner.wrote());
}

#[test]
fn inline_comment_outside_the_diff_is_rejected_before_any_write() {
    let fixture = fixture();
    let runner = RecordingRunner::replying(FILES_BODY);
    let ctx = context(&fixture, &runner);
    let request = PrReviewCommentRequest {
        repo: REPO.to_string(),
        number: NUMBER,
        path: FILE_PATH.to_string(),
        line: OUTSIDE_LINE,
        side: DiffSide::Right,
        start_line: None,
        start_side: None,
        body: DRAFT_BODY.to_string(),
    };
    let result = review_comment_create_with_state(&ctx, &state(OTHER), request);
    assert_eq!(status_of(result), ApiStatus::BadRequest.status());
    assert!(!runner.wrote());
}

#[test]
fn approving_your_own_pull_request_is_a_conflict_with_the_own_pr_reason() {
    let fixture = fixture();
    let runner = RecordingRunner::replying(REVIEW_REPLY);
    let ctx = context(&fixture, &runner);
    let result = review_submit_with_state(&ctx, &state(ME), review_request(ReviewEvent::Approve));
    let error = result.expect_err("an error reply");
    assert_eq!(error.status(), ApiStatus::Conflict.status());
    assert_eq!(error.reason, Some(PrRefusalReason::OwnPr));
    assert_eq!(runner.call_count(), 0);
}

#[test]
fn a_failed_submit_keeps_the_drafts_and_publishes_nothing() {
    let fixture = fixture();
    fixture
        .drafts
        .put(REPO, NUMBER, vec![draft(VALID_LINE)])
        .unwrap();
    let files = RecordingRunner::failing();
    let ctx = context(&fixture, &files);
    let result =
        review_submit_with_state(&ctx, &state(OTHER), review_request(ReviewEvent::Comment));
    assert!(result.is_err());
    assert_eq!(fixture.drafts.get(REPO, NUMBER).len(), 1);
}

#[test]
fn a_draft_outside_the_diff_blocks_the_submit() {
    let fixture = fixture();
    fixture
        .drafts
        .put(REPO, NUMBER, vec![draft(OUTSIDE_LINE)])
        .unwrap();
    let runner = RecordingRunner::replying(FILES_BODY);
    let ctx = context(&fixture, &runner);
    let result =
        review_submit_with_state(&ctx, &state(OTHER), review_request(ReviewEvent::Comment));
    assert_eq!(status_of(result), ApiStatus::BadRequest.status());
    assert!(!runner.wrote());
    assert_eq!(fixture.drafts.get(REPO, NUMBER).len(), 1);
}

#[test]
fn comment_ids_must_be_numeric() {
    assert_eq!(parse_comment_id("42").ok(), Some(42));
    for raw in ["state", "send", "", "4x2", "-1"] {
        assert_eq!(
            status_of(parse_comment_id(raw).map(Value::from)),
            ApiStatus::BadRequest.status()
        );
    }
}

#[test]
fn an_empty_comment_review_is_rejected() {
    let fixture = fixture();
    let runner = RecordingRunner::replying(REVIEW_REPLY);
    let ctx = context(&fixture, &runner);
    let result =
        review_submit_with_state(&ctx, &state(OTHER), review_request(ReviewEvent::Comment));
    assert_eq!(status_of(result), ApiStatus::BadRequest.status());
    assert_eq!(runner.call_count(), 0);
}
