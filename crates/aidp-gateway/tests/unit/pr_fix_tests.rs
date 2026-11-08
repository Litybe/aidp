use super::*;
use crate::models::constants::pr_fix::PR_FIX_SUMMARY_PREFIX;
use crate::models::constants::pr_fix_keys::{
    KEY_AUTHOR, KEY_BODY, KEY_COMMENTS, KEY_DIFF_HUNK, KEY_IS_RESOLVED, KEY_KIND, KEY_LINE,
    KEY_PATH, KEY_RESULT, KEY_TEXT, KEY_THREADS, KEY_THREAD_ID,
};
use crate::models::constants::pr_fix_limits::TAIL_LINE_SEPARATOR;
use crate::models::constants::pr_fix_recipe::{RECIPE_FILE_NAME, RECIPE_RELATIVE_DIR};
use crate::models::enums::{SessionEventKind, WorkspaceKind};
use crate::models::pr_workspace_holder::PrWorkspaceHolder;
use crate::pr_comment_state::PrCommentState;
use crate::pr_marks::PrMarks;
use crate::projects::Project;
use aidp_core::clock::{Clock, FixedClock};
use aidp_core::models::process::ProcessSpec;
use aidp_core::process_runner::SpawnedChild;
use aidp_integrations::models::pr_thread_mark::PrThreadMark;
use serde_json::json;
use std::collections::BTreeMap;
use std::process::Output;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

const THREADS_DOCUMENT: &str = include_str!("../fixtures/pr_fix/threads_document.json");
const ANSWERED_DOCUMENT: &str = include_str!("../fixtures/pr_fix/threads_answered.json");
const PARTIAL_DOCUMENT: &str = include_str!("../fixtures/pr_fix/threads_partial.json");
const REPLY_CHECK_FIELD: &str = "replyCheck";
const READ_FAILURE: &str = "github unreachable";
const RECIPE_STUB: &str = include_str!("../fixtures/pr_fix/recipe_stub.md");
const REPO: &str = "acme/svc";
const REPO_DIR: &str = "svc";
const NUMBER: u64 = 7;
const VIEWER: &str = "me";
const OTHER_AUTHOR: &str = "someone-else";
const TICKET_BRANCH: &str = "feat/ABC-1";
const TICKET_KEY: &str = "ABC-1";
const OTHER_BRANCH: &str = "alexis/cleanup";
const OPEN_ONE: &str = "PRRT_open_1";
const OPEN_TWO: &str = "PRRT_open_2";
const RESOLVED: &str = "PRRT_done";
const SESSION_ID: &str = "sess-1";
const NOW: &str = "2026-10-08T10:00:00Z";
const PUSHED_SHA: &str = "abc1234";
const PROJECT_ID: &str = "p1";
const GIT_PROGRAM: &str = "git";
const TRACK_FLAG: &str = "--track";
const EFFORT: &str = "high";
const NOTE: &str = "keep it small";
const ORIGIN_PREFIX: &str = "origin/";
const WORKTREE_ARG: &str = "worktree";
const REMOVE_ARG: &str = "remove";
const ADD_ARG: &str = "add";
const SHOW_REF_ARG: &str = "show-ref";
const EVENT_TYPE_KEY: &str = "type";
const EVENT_DATA_KEY: &str = "data";
const OTHER_PUSH_COMMAND: &str = "git push origin other";
const HEAD_PUSH_COMMAND: &str = "git push origin feat/ABC-1";
const BASH_RULE_OPEN: &str = "Bash(";
const BASH_RULE_CLOSE: char = ')';
const RULE_WILDCARD: char = '*';
const HUNK: &str = "@@ -1,2 +1,2 @@
-old
+new";
const HUNK_THREAD: &str = "PRRT_hunk";
const NOT_RELEVANT_THREAD: &str = "PRRT_skip";
const MISSING_THREAD: &str = "PRRT_missing";
const HOLDER_STAGE: &str = "implement";
const LIVE_RUN: &str = "20261008-0002";
const DEAD_RUN: &str = "20261008-0003";
const HOLDER_ACQUIRED_AT: &str = "2026-10-08T09:00:00.000Z";
const IGNORED_KIND: &str = "tool_use";
const FIRST_TEXT: &str = "first";
const LAST_TEXT: &str = "last";
const IGNORED_TEXT: &str = "ignored";
const HUNK_PATH: &str = "src/one.rs";
const HUNK_LINE: u64 = 3;
const FIRST_AUTHOR: &str = "alice";
const SECOND_AUTHOR: &str = "bob";

/// Thread documents served in turn: the first read is the start, later reads the end.
struct ThreadsScript {
    reads: AtomicUsize,
    end_document: Option<&'static str>,
}

#[derive(Clone)]
struct FakeGithub {
    pull: PrFixPull,
    script: Arc<ThreadsScript>,
}

impl FakeGithub {
    fn new(pull: PrFixPull) -> Self {
        Self::ending_with(pull, Some(THREADS_DOCUMENT))
    }

    /// A github whose end-of-session read returns `end_document`, or fails when `None`.
    fn ending_with(pull: PrFixPull, end_document: Option<&'static str>) -> Self {
        Self {
            pull,
            script: Arc::new(ThreadsScript {
                reads: AtomicUsize::new(0),
                end_document,
            }),
        }
    }
}

impl PrFixGithub for FakeGithub {
    fn pull(&self, _repo: &str, _number: u64) -> Result<PrFixPull, IntegrationError> {
        Ok(self.pull.clone())
    }

    fn threads(&self, _repo: &str, _number: u64) -> Result<Value, IntegrationError> {
        let text = match self.script.reads.fetch_add(1, Ordering::SeqCst) {
            0 => THREADS_DOCUMENT,
            _ => self
                .script
                .end_document
                .ok_or_else(|| IntegrationError::Upstream(READ_FAILURE.to_string()))?,
        };
        Ok(serde_json::from_str(text).unwrap())
    }
}

#[derive(Default)]
struct FakeHost {
    holder: Option<String>,
    queued: bool,
    started: Mutex<Vec<StartRequest>>,
    cancelled: Mutex<Vec<String>>,
    hook: Mutex<Option<SessionEndCallback>>,
}

impl FakeHost {
    fn end(&self, outcome: SessionOutcome) {
        let hook = self.hook.lock().unwrap().take().expect("a registered hook");
        hook(outcome);
    }

    fn only_start(&self) -> StartRequest {
        let started = self.started.lock().unwrap();
        assert_eq!(started.len(), 1);
        started[0].clone()
    }
}

impl PrFixHost for FakeHost {
    fn busy_holder(&self, _workspace_key: &str) -> Option<String> {
        self.holder.clone()
    }

    fn start(&self, request: StartRequest) -> anyhow::Result<StartedSession> {
        self.started.lock().unwrap().push(request);
        Ok(StartedSession {
            id: SESSION_ID.to_string(),
            queued: self.queued,
        })
    }

    fn cancel_queued(&self, queue_id: &str) {
        self.cancelled.lock().unwrap().push(queue_id.to_string());
    }

    fn on_end(&self, _session_id: &str, hook: SessionEndCallback) {
        *self.hook.lock().unwrap() = Some(hook);
    }
}

#[derive(Default)]
struct RecordingRunner {
    calls: Mutex<Vec<Vec<String>>>,
}

impl RecordingRunner {
    fn git_calls(&self) -> Vec<Vec<String>> {
        self.calls.lock().unwrap().clone()
    }

    fn has_call(&self, words: &[&str]) -> bool {
        self.git_calls()
            .iter()
            .any(|call| words.iter().all(|word| call.iter().any(|arg| arg == word)))
    }
}

fn exit_status(code: i32) -> std::process::ExitStatus {
    #[cfg(windows)]
    {
        use std::os::windows::process::ExitStatusExt;
        std::process::ExitStatus::from_raw(code as u32)
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        std::process::ExitStatus::from_raw(code)
    }
}

impl ProcessRunner for RecordingRunner {
    fn output(&self, spec: &ProcessSpec) -> std::io::Result<Output> {
        assert_eq!(spec.program, GIT_PROGRAM);
        self.calls.lock().unwrap().push(spec.args.clone());
        let missing_ref = spec.args.iter().any(|arg| arg == SHOW_REF_ARG);
        Ok(Output {
            status: exit_status(i32::from(missing_ref)),
            stdout: Vec::new(),
            stderr: Vec::new(),
        })
    }

    fn spawn(&self, _spec: &ProcessSpec) -> std::io::Result<Box<dyn SpawnedChild>> {
        Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
    }
}

struct Harness {
    _root: tempfile::TempDir,
    vault: PathBuf,
    skill_root: PathBuf,
    project: Project,
    marks: PrMarks,
    comment_state: PrCommentState,
    bus: Arc<Bus>,
    runner: Arc<RecordingRunner>,
    clock: FixedClock,
    live_runs: Vec<String>,
}

fn harness() -> Harness {
    let root = tempfile::tempdir().unwrap();
    let vault = root.path().join("vault");
    let skill_root = root.path().join("skill");
    let project_dir = root.path().join(REPO_DIR);
    let state_dir = root.path().join("state");
    let recipe_dir = skill_root.join(RECIPE_RELATIVE_DIR);
    for dir in [&vault, &project_dir, &state_dir, &recipe_dir] {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(recipe_dir.join(RECIPE_FILE_NAME), RECIPE_STUB).unwrap();
    Harness {
        vault,
        skill_root,
        project: Project {
            id: PROJECT_ID.to_string(),
            name: REPO_DIR.to_string(),
            path: project_dir.display().to_string(),
            runs_count: 0,
            test_checkout: None,
        },
        marks: PrMarks::new(&state_dir),
        comment_state: PrCommentState::new(&state_dir),
        bus: Arc::new(Bus::new()),
        runner: Arc::new(RecordingRunner::default()),
        clock: FixedClock::from_rfc3339(NOW),
        live_runs: Vec::new(),
        _root: root,
    }
}

fn pull(branch: &str) -> PrFixPull {
    PrFixPull {
        author: Some(VIEWER.to_string()),
        viewer: Some(VIEWER.to_string()),
        state: PrState::Open,
        head_branch: branch.to_string(),
    }
}

fn request(threads: &[&str]) -> PrFixRequest {
    PrFixRequest {
        repo: REPO.to_string(),
        number: NUMBER,
        threads: threads.iter().map(|id| id.to_string()).collect(),
        points: Vec::new(),
        note: None,
        model: None,
        effort: None,
    }
}

fn attempt(
    harness: &Harness,
    github: &FakeGithub,
    host: &FakeHost,
    request: PrFixRequest,
) -> Result<Value, PrRouteError> {
    let live_runs = harness.live_runs.clone();
    let ports = PrFixPorts {
        github: Arc::new(github.clone()),
        run_is_live: Arc::new(move |run_id| live_runs.iter().any(|live| live == run_id)),
        host,
        runner: harness.runner.clone(),
        sleeper: Arc::new(|_| {}),
        bus: harness.bus.clone(),
        clock: &harness.clock,
        marks: &harness.marks,
        comment_state: &harness.comment_state,
        vault_root: &harness.vault,
        skill_root: &harness.skill_root,
        project: &harness.project,
    };
    fix_pull_request(&ports, request)
}

fn run(harness: &Harness, github: &FakeGithub, host: &FakeHost, request: PrFixRequest) -> Value {
    match attempt(harness, github, host, request) {
        Ok(reply) => reply,
        Err(error) => panic!(
            "fix refused with {}: {}",
            error.status(),
            error.error.message
        ),
    }
}

fn events_of(harness: &Harness, phase: PrFixPhase) -> Vec<PrFixSessionEvent> {
    harness
        .bus
        .recent(100, 0)
        .into_iter()
        .filter(|event| event[EVENT_TYPE_KEY] == PR_FIX_EVENT)
        .map(|event| {
            serde_json::from_value::<PrFixSessionEvent>(event[EVENT_DATA_KEY].clone()).unwrap()
        })
        .filter(|event| event.phase == phase)
        .collect()
}

/// Runs a fix on the other-branch PR, ends it with a pushed summary and returns the ended event.
fn ended_after_reading(github: FakeGithub) -> PrFixSessionEvent {
    let harness = harness();
    let host = FakeHost::default();
    run(&harness, &github, &host, request(&[OPEN_ONE, OPEN_TWO]));
    host.end(success(summary_tail(Some(PUSHED_SHA))));
    let mut ended = events_of(&harness, PrFixPhase::Ended);
    assert_eq!(ended.len(), 1);
    ended.remove(0)
}

fn ticket_worktree(harness: &Harness) -> PathBuf {
    let path = worktrees::worktree_path(&harness.vault, REPO_DIR, TICKET_KEY);
    std::fs::create_dir_all(&path).unwrap();
    path
}

fn summary_tail(pushed_sha: Option<&str>) -> String {
    let summary = PrFixSummary {
        fixed: vec![OPEN_ONE.to_string()],
        needs_reviewer: vec![OPEN_TWO.to_string()],
        not_relevant: Vec::new(),
        pushed_sha: pushed_sha.map(str::to_string),
        assumptions: vec!["kept the public name".to_string()],
    };
    format!(
        "done\n{PR_FIX_SUMMARY_PREFIX}{}\n",
        serde_json::to_string(&summary).unwrap()
    )
}

fn success(tail: String) -> SessionOutcome {
    SessionOutcome {
        succeeded: true,
        error: None,
        transcript_tail: tail,
    }
}

#[test]
fn pr_fix_refuses_a_pull_request_not_authored_by_the_viewer() {
    let harness = harness();
    let mut foreign = pull(TICKET_BRANCH);
    foreign.author = Some(OTHER_AUTHOR.to_string());
    let host = FakeHost::default();
    let error = attempt(
        &harness,
        &FakeGithub::new(foreign),
        &host,
        request(&[OPEN_ONE]),
    )
    .unwrap_err();
    assert_eq!(error.status(), 403);
    assert_eq!(error.reason, Some(PrRefusalReason::NotMine));
    assert!(host.started.lock().unwrap().is_empty());
}

#[test]
fn pr_fix_refuses_a_merged_or_closed_pull_request() {
    for state in [PrState::Merged, PrState::Closed] {
        let harness = harness();
        let mut finished = pull(TICKET_BRANCH);
        finished.state = state;
        let host = FakeHost::default();
        let error = attempt(
            &harness,
            &FakeGithub::new(finished),
            &host,
            request(&[OPEN_ONE]),
        )
        .unwrap_err();
        assert_eq!(error.status(), 409);
        assert!(host.started.lock().unwrap().is_empty());
    }
}

#[test]
fn pr_fix_refuses_a_busy_workspace_and_names_the_holder() {
    let harness = harness();
    ticket_worktree(&harness);
    let host = FakeHost {
        holder: Some("session s-9".to_string()),
        ..Default::default()
    };
    let error = attempt(
        &harness,
        &FakeGithub::new(pull(TICKET_BRANCH)),
        &host,
        request(&[OPEN_ONE]),
    )
    .unwrap_err();
    assert_eq!(error.status(), 409);
    assert!(error.error.message.contains("session s-9"));
    assert!(host.started.lock().unwrap().is_empty());
}

#[test]
fn pr_fix_drops_a_resolved_thread_and_keeps_the_open_ones() {
    let harness = harness();
    ticket_worktree(&harness);
    let host = FakeHost::default();
    let reply = run(
        &harness,
        &FakeGithub::new(pull(TICKET_BRANCH)),
        &host,
        request(&[OPEN_ONE, RESOLVED, OPEN_TWO]),
    );
    let response: PrFixResponse = serde_json::from_value(reply).unwrap();
    assert_eq!(response.threads, vec![OPEN_ONE, OPEN_TWO]);
    assert_eq!(response.dropped_threads, vec![RESOLVED]);
    let prompt = host.only_start().prompt.unwrap();
    assert!(prompt.contains(OPEN_ONE) && prompt.contains("add a test"));
    assert!(!prompt.contains(RESOLVED) && !prompt.contains("already handled"));
}

#[test]
fn pr_fix_refuses_when_every_selected_thread_is_resolved() {
    let harness = harness();
    let host = FakeHost::default();
    let error = attempt(
        &harness,
        &FakeGithub::new(pull(TICKET_BRANCH)),
        &host,
        request(&[RESOLVED]),
    )
    .unwrap_err();
    assert_eq!(error.status(), 409);
    assert!(host.started.lock().unwrap().is_empty());
}

#[test]
fn pr_fix_uses_the_ticket_worktree_for_a_ticket_branch() {
    let harness = harness();
    let path = ticket_worktree(&harness);
    let host = FakeHost::default();
    let reply = run(
        &harness,
        &FakeGithub::new(pull(TICKET_BRANCH)),
        &host,
        request(&[OPEN_ONE]),
    );
    let response: PrFixResponse = serde_json::from_value(reply).unwrap();
    assert_eq!(response.workspace_kind, WorkspaceKind::Ticket);
    assert_eq!(PathBuf::from(&response.workspace), path);
    assert!(harness.runner.git_calls().is_empty());
    let start = host.only_start();
    assert_eq!(
        start.workspace.unwrap().path,
        Some(path.display().to_string())
    );
}

#[test]
fn pr_fix_creates_a_pr_worktree_for_any_other_branch() {
    let harness = harness();
    let host = FakeHost::default();
    let reply = run(
        &harness,
        &FakeGithub::new(pull(OTHER_BRANCH)),
        &host,
        request(&[OPEN_ONE]),
    );
    let response: PrFixResponse = serde_json::from_value(reply).unwrap();
    let expected = worktrees::pr_worktree_path(&harness.vault, REPO_DIR, NUMBER);
    assert_eq!(response.workspace_kind, WorkspaceKind::PrWorktree);
    assert_eq!(PathBuf::from(&response.workspace), expected);
    let remote_branch = format!("{ORIGIN_PREFIX}{OTHER_BRANCH}");
    assert!(harness.runner.has_call(&[
        WORKTREE_ARG,
        ADD_ARG,
        TRACK_FLAG,
        OTHER_BRANCH,
        &remote_branch
    ]));
}

#[test]
fn pr_fix_starts_an_autonomous_prompt_session_limited_to_the_head_branch() {
    let harness = harness();
    ticket_worktree(&harness);
    let host = FakeHost::default();
    let mut body = request(&[OPEN_ONE]);
    body.effort = Some(EFFORT.to_string());
    body.note = Some(NOTE.to_string());
    run(&harness, &FakeGithub::new(pull(TICKET_BRANCH)), &host, body);
    let start = host.only_start();
    assert_eq!(start.engine.as_deref(), Some(Engine::Prompt.as_str()));
    assert_eq!(start.model.as_deref(), Some(PR_FIX_DEFAULT_MODEL));
    assert_eq!(start.effort.as_deref(), Some(EFFORT));
    assert_eq!(start.auto_close, Some(true));
    assert_eq!(
        start.permission_mode.as_deref(),
        Some(PermissionMode::DontAsk.as_str())
    );
    assert_eq!(
        start.env.get(ALLOWED_PUSH_BRANCH_ENV).map(String::as_str),
        Some(TICKET_BRANCH)
    );
    assert_eq!(
        start.env.get(AUTONOMOUS_ENV).map(String::as_str),
        Some(AUTONOMOUS_ENABLED)
    );
    let origin = start.origin.unwrap();
    assert_eq!(origin.kind, OriginKind::PrFix.as_str());
    assert_eq!(origin.r#ref.as_deref(), Some("acme/svc#7"));
    assert!(start.prompt.unwrap().contains(NOTE));
}

#[test]
fn pr_fix_cancels_a_launch_that_got_queued() {
    let harness = harness();
    ticket_worktree(&harness);
    let host = FakeHost {
        queued: true,
        ..Default::default()
    };
    let error = attempt(
        &harness,
        &FakeGithub::new(pull(TICKET_BRANCH)),
        &host,
        request(&[OPEN_ONE]),
    )
    .unwrap_err();
    assert_eq!(error.status(), 409);
    assert_eq!(
        *host.cancelled.lock().unwrap(),
        vec![SESSION_ID.to_string()]
    );
    assert!(harness.marks.all().is_empty());
}

#[test]
fn pr_fix_writes_the_marks_and_hands_the_threads() {
    let harness = harness();
    ticket_worktree(&harness);
    let host = FakeHost::default();
    run(
        &harness,
        &FakeGithub::new(pull(TICKET_BRANCH)),
        &host,
        request(&[OPEN_ONE, OPEN_TWO]),
    );
    let marks = harness.marks.all();
    let mark = &marks[&mark_key(REPO, NUMBER)];
    assert_eq!(mark.fix_session.as_deref(), Some(SESSION_ID));
    assert_eq!(
        mark.fix_at.as_deref(),
        Some(harness.clock.now_rfc3339().as_str())
    );
    let expected_thread_mark = PrThreadMark {
        fix_session: SESSION_ID.to_string(),
        at: harness.clock.now_rfc3339(),
    };
    let expected_threads = BTreeMap::from([
        (OPEN_ONE.to_string(), expected_thread_mark.clone()),
        (OPEN_TWO.to_string(), expected_thread_mark),
    ]);
    assert_eq!(mark.threads, expected_threads);
    let handed = harness.comment_state.read(REPO, NUMBER).unwrap().handed;
    assert_eq!(handed, vec![OPEN_ONE.to_string(), OPEN_TWO.to_string()]);
    assert_eq!(events_of(&harness, PrFixPhase::Started).len(), 1);
}

#[test]
fn pr_fix_ended_event_carries_the_summary_and_removes_the_pr_worktree() {
    let harness = harness();
    let host = FakeHost::default();
    run(
        &harness,
        &FakeGithub::new(pull(OTHER_BRANCH)),
        &host,
        request(&[OPEN_ONE, OPEN_TWO]),
    );
    host.end(success(summary_tail(Some(PUSHED_SHA))));
    let ended = events_of(&harness, PrFixPhase::Ended);
    assert_eq!(ended.len(), 1);
    assert_eq!(ended[0].fixed, Some(vec![OPEN_ONE.to_string()]));
    assert_eq!(ended[0].needs_reviewer, Some(vec![OPEN_TWO.to_string()]));
    assert_eq!(
        ended[0].assumptions,
        Some(vec!["kept the public name".to_string()])
    );
    assert_eq!(ended[0].pushed_sha.as_deref(), Some(PUSHED_SHA));
    assert!(!ended[0].summary_missing);
    assert_eq!(ended[0].worktree_kept, None);
    assert!(harness.runner.has_call(&[WORKTREE_ARG, REMOVE_ARG]));
}

#[test]
fn pr_fix_ended_event_flags_a_missing_summary_without_failing() {
    let harness = harness();
    let host = FakeHost::default();
    run(
        &harness,
        &FakeGithub::new(pull(OTHER_BRANCH)),
        &host,
        request(&[OPEN_ONE]),
    );
    host.end(success("no summary here".to_string()));
    let ended = events_of(&harness, PrFixPhase::Ended);
    assert!(ended[0].summary_missing);
    assert_eq!(ended[0].error, None);
    assert!(ended[0].worktree_kept.is_some());
    assert!(!harness.runner.has_call(&[WORKTREE_ARG, REMOVE_ARG]));
}

#[test]
fn pr_fix_keeps_the_pr_worktree_when_the_session_fails_before_the_push() {
    let harness = harness();
    let host = FakeHost::default();
    run(
        &harness,
        &FakeGithub::new(pull(OTHER_BRANCH)),
        &host,
        request(&[OPEN_ONE]),
    );
    let expected = worktrees::pr_worktree_path(&harness.vault, REPO_DIR, NUMBER);
    host.end(SessionOutcome {
        succeeded: false,
        error: Some("boom".to_string()),
        transcript_tail: summary_tail(None),
    });
    let ended = events_of(&harness, PrFixPhase::Ended);
    assert_eq!(ended[0].error.as_deref(), Some("boom"));
    assert_eq!(ended[0].worktree_kept, Some(expected.display().to_string()));
    assert!(!harness.runner.has_call(&[WORKTREE_ARG, REMOVE_ARG]));
}

#[test]
fn pr_fix_never_removes_a_ticket_worktree() {
    let harness = harness();
    let path = ticket_worktree(&harness);
    let host = FakeHost::default();
    run(
        &harness,
        &FakeGithub::new(pull(TICKET_BRANCH)),
        &host,
        request(&[OPEN_ONE]),
    );
    host.end(success(summary_tail(Some(PUSHED_SHA))));
    let ended = events_of(&harness, PrFixPhase::Ended);
    assert_eq!(ended[0].pushed_sha.as_deref(), Some(PUSHED_SHA));
    assert_eq!(ended[0].worktree_kept, None);
    assert!(path.is_dir());
    assert!(harness.runner.git_calls().is_empty());
}

#[test]
fn pr_fix_transcript_tail_keeps_assistant_text_and_turn_results() {
    let events = vec![
        json!({ (KEY_KIND): SessionEventKind::Text.as_str(), (KEY_TEXT): FIRST_TEXT }),
        json!({ (KEY_KIND): IGNORED_KIND, (KEY_TEXT): IGNORED_TEXT }),
        json!({ (KEY_KIND): SessionEventKind::TurnEnd.as_str(), (KEY_RESULT): LAST_TEXT }),
    ];
    assert_eq!(
        transcript_tail(&events),
        format!("{FIRST_TEXT}{TAIL_LINE_SEPARATOR}{LAST_TEXT}")
    );
}

/// Whether an allow rule `Bash(<prefix>*)` or `Bash(<command>)` covers `command`.
fn rule_permits(rule: &str, command: &str) -> bool {
    let Some(body) = rule
        .strip_prefix(BASH_RULE_OPEN)
        .and_then(|rest| rest.strip_suffix(BASH_RULE_CLOSE))
    else {
        return false;
    };
    match body.strip_suffix(RULE_WILDCARD) {
        Some(prefix) => command.starts_with(prefix),
        None => body == command,
    }
}

#[test]
fn pr_fix_allow_list_permits_the_head_branch_push_and_refuses_another_branch() {
    let rules = allowed_tools(TICKET_BRANCH).unwrap();
    assert!(rules
        .iter()
        .any(|rule| rule_permits(rule, HEAD_PUSH_COMMAND)));
    assert!(!rules
        .iter()
        .any(|rule| rule_permits(rule, OTHER_PUSH_COMMAND)));
}

#[test]
fn pr_fix_thread_input_reads_the_diff_hunk_of_the_first_comment() {
    let document = json!({
        (KEY_THREADS): [{
            (KEY_THREAD_ID): HUNK_THREAD,
            (KEY_IS_RESOLVED): false,
            (KEY_PATH): HUNK_PATH,
            (KEY_LINE): HUNK_LINE,
            (KEY_COMMENTS): [
                { (KEY_AUTHOR): FIRST_AUTHOR, (KEY_BODY): FIRST_TEXT, (KEY_DIFF_HUNK): HUNK },
                { (KEY_AUTHOR): SECOND_AUTHOR, (KEY_BODY): LAST_TEXT, (KEY_DIFF_HUNK): IGNORED_TEXT }
            ]
        }]
    });
    let (open, dropped) = split_selection(&document, &[HUNK_THREAD.to_string()]);
    assert!(dropped.is_empty());
    assert_eq!(open[0].diff_hunk, HUNK);
}

#[test]
fn pr_fix_ended_event_carries_not_relevant_and_flags_unreported_threads() {
    let harness = harness();
    let host = FakeHost::default();
    run(
        &harness,
        &FakeGithub::new(pull(OTHER_BRANCH)),
        &host,
        request(&[OPEN_ONE, OPEN_TWO]),
    );
    let summary = PrFixSummary {
        fixed: vec![OPEN_ONE.to_string()],
        not_relevant: vec![NOT_RELEVANT_THREAD.to_string()],
        pushed_sha: Some(PUSHED_SHA.to_string()),
        ..Default::default()
    };
    host.end(success(format!(
        "{PR_FIX_SUMMARY_PREFIX}{}
",
        serde_json::to_string(&summary).unwrap()
    )));
    let ended = events_of(&harness, PrFixPhase::Ended);
    assert_eq!(
        ended[0].not_relevant,
        Some(vec![NOT_RELEVANT_THREAD.to_string()])
    );
    assert_eq!(ended[0].unreported, Some(vec![OPEN_TWO.to_string()]));
}

#[test]
fn pr_fix_ended_event_reports_all_answered_when_the_author_replied_to_every_thread() {
    let ended = ended_after_reading(FakeGithub::ending_with(
        pull(OTHER_BRANCH),
        Some(ANSWERED_DOCUMENT),
    ));
    assert_eq!(ended.reply_check, PrFixReplyCheck::AllAnswered);
    assert_eq!(ended.unanswered, None);
}

#[test]
fn pr_fix_ended_event_names_the_thread_without_a_new_author_reply() {
    let ended = ended_after_reading(FakeGithub::ending_with(
        pull(OTHER_BRANCH),
        Some(PARTIAL_DOCUMENT),
    ));
    assert_eq!(ended.reply_check, PrFixReplyCheck::Unanswered);
    assert_eq!(ended.unanswered, Some(vec![OPEN_TWO.to_string()]));
}

#[test]
fn pr_fix_ended_event_counts_no_new_comment_as_unanswered_even_on_success() {
    let ended = ended_after_reading(FakeGithub::new(pull(OTHER_BRANCH)));
    assert_eq!(ended.reply_check, PrFixReplyCheck::Unanswered);
    assert_eq!(
        ended.unanswered,
        Some(vec![OPEN_ONE.to_string(), OPEN_TWO.to_string()])
    );
}

#[test]
fn pr_fix_ended_event_is_unverified_when_github_cannot_be_read() {
    let ended = ended_after_reading(FakeGithub::ending_with(pull(OTHER_BRANCH), None));
    assert_eq!(ended.reply_check, PrFixReplyCheck::Unverified);
    assert_eq!(ended.unanswered, None);
}

#[test]
fn pr_fix_started_event_carries_not_checked() {
    let harness = harness();
    let host = FakeHost::default();
    run(
        &harness,
        &FakeGithub::new(pull(OTHER_BRANCH)),
        &host,
        request(&[OPEN_ONE]),
    );
    let started = events_of(&harness, PrFixPhase::Started);
    assert_eq!(started[0].reply_check, PrFixReplyCheck::NotChecked);
    assert_eq!(started[0].unanswered, None);
}

#[test]
fn pr_fix_session_event_without_reply_fields_still_deserializes() {
    let event = PrFixSessionEvent {
        repo: REPO.to_string(),
        number: NUMBER,
        session_id: SESSION_ID.to_string(),
        phase: PrFixPhase::Ended,
        threads: Vec::new(),
        fixed: None,
        needs_reviewer: None,
        not_relevant: None,
        unreported: None,
        assumptions: None,
        pushed_sha: None,
        worktree_kept: None,
        error: None,
        summary_missing: false,
        unanswered: None,
        reply_check: PrFixReplyCheck::Unanswered,
    };
    let mut payload = serde_json::to_value(&event).unwrap();
    payload.as_object_mut().unwrap().remove(REPLY_CHECK_FIELD);
    let restored: PrFixSessionEvent = serde_json::from_value(payload).unwrap();
    assert_eq!(restored.reply_check, PrFixReplyCheck::NotChecked);
}

#[test]
fn pr_fix_summary_unreported_ignores_threads_placed_in_a_bucket() {
    let summary = PrFixSummary {
        needs_reviewer: vec![OPEN_ONE.to_string()],
        ..Default::default()
    };
    let selected = vec![OPEN_ONE.to_string(), MISSING_THREAD.to_string()];
    assert_eq!(
        summary.unreported(&selected),
        vec![MISSING_THREAD.to_string()]
    );
}

fn write_run_holder(harness: &Harness, run_id: &str) {
    let holder = PrWorkspaceHolder {
        stage: HOLDER_STAGE.to_string(),
        run_id: Some(run_id.to_string()),
        session_id: None,
        acquired_at: HOLDER_ACQUIRED_AT.to_string(),
    };
    pr_workspace_holder::write(&harness.vault, REPO_DIR, NUMBER, &holder).unwrap();
}

/// A vault holder naming a live run keeps the workspace busy and stays recorded.
#[test]
fn pr_fix_refuses_when_the_vault_holder_names_a_live_run() {
    let mut harness = harness();
    harness.live_runs.push(LIVE_RUN.to_string());
    write_run_holder(&harness, LIVE_RUN);
    let host = FakeHost::default();
    let error = attempt(
        &harness,
        &FakeGithub::new(pull(OTHER_BRANCH)),
        &host,
        request(&[OPEN_ONE]),
    )
    .unwrap_err();
    assert_eq!(error.status(), 409);
    assert!(error.error.message.contains(LIVE_RUN));
    assert!(pr_workspace_holder::read(&harness.vault, REPO_DIR, NUMBER).is_some());
    assert!(host.started.lock().unwrap().is_empty());
}

/// A vault holder naming a dead run is ignored, purged, and the fix proceeds.
#[test]
fn pr_fix_purges_a_vault_holder_whose_run_is_dead_and_proceeds() {
    let harness = harness();
    write_run_holder(&harness, DEAD_RUN);
    let host = FakeHost::default();
    run(
        &harness,
        &FakeGithub::new(pull(OTHER_BRANCH)),
        &host,
        request(&[OPEN_ONE]),
    );
    assert!(pr_workspace_holder::read(&harness.vault, REPO_DIR, NUMBER).is_none());
    assert_eq!(host.started.lock().unwrap().len(), 1);
}
