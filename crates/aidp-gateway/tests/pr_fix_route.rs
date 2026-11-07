//! `POST /api/prs/fix` end to end with a scripted GitHub, session host and git runner.

use aidp_core::clock::FixedClock;
use aidp_core::models::process::ProcessSpec;
use aidp_core::process_runner::{ProcessRunner, SpawnedChild};
use aidp_gateway::bus::Bus;
use aidp_gateway::flows::lock::{LocalLock, WorkspaceGuard, WorkspaceLock};
use aidp_gateway::models::constants::pr_fix::PR_FIX_EVENT;
use aidp_gateway::models::constants::pr_fix::PR_FIX_SUMMARY_PREFIX;
use aidp_gateway::models::constants::pr_fix_keys::{KEY_COMMENTS, KEY_THREADS, KEY_THREAD_ID};
use aidp_gateway::models::constants::pr_fix_recipe::{RECIPE_FILE_NAME, RECIPE_RELATIVE_DIR};
use aidp_gateway::models::enums::pr_fix_phase::PrFixPhase;
use aidp_gateway::models::enums::pr_fix_reply_check::PrFixReplyCheck;
use aidp_gateway::models::enums::pr_refusal_reason::PrRefusalReason;
use aidp_gateway::models::pr_fix_request::PrFixRequest;
use aidp_gateway::models::pr_fix_session_event::PrFixSessionEvent;
use aidp_gateway::models::pr_fix_summary::PrFixSummary;
use aidp_gateway::models::pr_workspace_holder::PrWorkspaceHolder;
use aidp_gateway::pr_comment_state::PrCommentState;
use aidp_gateway::pr_fix::{
    fix_pull_request, PrFixGithub, PrFixHost, PrFixPorts, PrFixPull, SessionEndCallback,
    SessionOutcome, StartedSession,
};
use aidp_gateway::pr_marks::PrMarks;
use aidp_gateway::pr_workspace_holder;
use aidp_gateway::projects::Project;
use aidp_gateway::prs::PrRouteError;
use aidp_gateway::sessions::StartRequest;
use aidp_gateway::worktrees;
use aidp_integrations::pr::PrState;
use aidp_integrations::IntegrationError;
use aidp_testkit::TempWorkspace;
use serde_json::Value;
use std::path::PathBuf;
use std::process::Output;
use std::sync::{Arc, Mutex};

const THREADS_DOCUMENT: &str = include_str!("fixtures/pr_fix/route/threads.json");
const RECIPE_STUB: &str = include_str!("fixtures/pr_fix/recipe_stub.md");
const REPO: &str = "acme/svc";
const REPO_DIR: &str = "svc";
const NUMBER: u64 = 7;
const VIEWER: &str = "me";
const OTHER_AUTHOR: &str = "someone-else";
const OTHER_BRANCH: &str = "alexis/cleanup";
const TICKET_BRANCH: &str = "feat/ABC-1";
const TICKET_KEY: &str = "ABC-1";
const THREAD_A: &str = "PRRT_a";
const THREAD_B: &str = "PRRT_b";
const THREAD_C: &str = "PRRT_c";
const THREAD_D: &str = "PRRT_d";
const ALL_THREADS: [&str; 4] = [THREAD_A, THREAD_B, THREAD_C, THREAD_D];
const SESSION_ID: &str = "sess-1";
const NOW: &str = "2026-10-08T10:00:00Z";
const PUSHED_SHA: &str = "abc1234";
const PROJECT_ID: &str = "p1";
const STAGE_HOLDER: &str = "workflow stage implement of run 20261008-0001";
const STAGE_NAME: &str = "implement";
const HOLDER_RUN_ID: &str = "20261008-0002";
const HOLDER_ACQUIRED_AT: &str = "2026-10-08T09:00:00.000Z";
const LOCK_KEY: &str = "held-workspace";
const FAILURE_TEXT: &str = "session crashed";
const GIT_PROGRAM: &str = "git";
const SHOW_REF_ARG: &str = "show-ref";
const WORKTREE_ARG: &str = "worktree";
const REMOVE_ARG: &str = "remove";
const EVENT_TYPE_KEY: &str = "type";
const EVENT_DATA_KEY: &str = "data";
const REASON_KEY: &str = "reason";
const ERROR_KEY: &str = "error";
const NOT_MINE_REASON: &str = "not-mine";
const STATUS_FORBIDDEN: u16 = 403;
const STATUS_CONFLICT: u16 = 409;
const EVENT_LIMIT: usize = 100;
const REPLY_BODY: &str = "done";

/// A comment the viewer left on a thread.
#[derive(serde::Serialize)]
struct ViewerReply {
    author: &'static str,
    body: &'static str,
}

/// A scripted `gh`: serves one pull request and the thread document, records thread replies.
struct FakeGh {
    pull: PrFixPull,
    replies: Mutex<Vec<String>>,
}

impl FakeGh {
    fn new(pull: PrFixPull) -> Arc<Self> {
        Arc::new(FakeGh {
            pull,
            replies: Mutex::new(Vec::new()),
        })
    }

    fn replied(&self) -> Vec<String> {
        self.replies.lock().unwrap().clone()
    }

    /// The thread document where each replied thread gained a comment by the viewer.
    fn document_with_replies(&self) -> Value {
        let mut document: Value = serde_json::from_str(THREADS_DOCUMENT).unwrap();
        let replied = self.replied();
        let threads = document[KEY_THREADS].as_array_mut().unwrap();
        for thread in threads {
            let id = thread[KEY_THREAD_ID].as_str().unwrap().to_string();
            if replied.contains(&id) {
                let reply = serde_json::to_value(ViewerReply {
                    author: VIEWER,
                    body: REPLY_BODY,
                })
                .unwrap();
                thread[KEY_COMMENTS].as_array_mut().unwrap().push(reply);
            }
        }
        document
    }
}

struct GhPort(Arc<FakeGh>);

impl PrFixGithub for GhPort {
    fn pull(&self, _repo: &str, _number: u64) -> Result<PrFixPull, IntegrationError> {
        Ok(self.0.pull.clone())
    }

    fn threads(&self, _repo: &str, _number: u64) -> Result<Value, IntegrationError> {
        Ok(self.0.document_with_replies())
    }
}

/// A scripted session host; a finished session replies to every thread its prompt holds.
struct FakeSessions {
    gh: Arc<FakeGh>,
    holder: Option<String>,
    started: Mutex<Vec<StartRequest>>,
    hook: Mutex<Option<SessionEndCallback>>,
}

impl FakeSessions {
    fn new(gh: &Arc<FakeGh>, holder: Option<&str>) -> Self {
        FakeSessions {
            gh: gh.clone(),
            holder: holder.map(str::to_string),
            started: Mutex::new(Vec::new()),
            hook: Mutex::new(None),
        }
    }

    fn prompt(&self) -> String {
        let started = self.started.lock().unwrap();
        assert_eq!(started.len(), 1);
        started[0].prompt.clone().unwrap()
    }

    fn started_count(&self) -> usize {
        self.started.lock().unwrap().len()
    }

    fn finish_with_summary(&self) {
        let prompt = self.prompt();
        let handled: Vec<String> = ALL_THREADS
            .iter()
            .filter(|id| prompt.contains(**id))
            .map(|id| id.to_string())
            .collect();
        self.gh.replies.lock().unwrap().extend(handled.clone());
        let summary = PrFixSummary {
            fixed: handled,
            pushed_sha: Some(PUSHED_SHA.to_string()),
            ..Default::default()
        };
        let tail = format!(
            "done\n{PR_FIX_SUMMARY_PREFIX}{}\n",
            serde_json::to_string(&summary).unwrap()
        );
        self.end(SessionOutcome {
            succeeded: true,
            error: None,
            transcript_tail: tail,
        });
    }

    fn end(&self, outcome: SessionOutcome) {
        let hook = self.hook.lock().unwrap().take().expect("a registered hook");
        hook(outcome);
    }
}

impl PrFixHost for FakeSessions {
    fn busy_holder(&self, _workspace_key: &str) -> Option<String> {
        self.holder.clone()
    }

    fn start(&self, request: StartRequest) -> anyhow::Result<StartedSession> {
        self.started.lock().unwrap().push(request);
        Ok(StartedSession {
            id: SESSION_ID.to_string(),
            queued: false,
        })
    }

    fn cancel_queued(&self, _queue_id: &str) {}

    fn on_end(&self, _session_id: &str, hook: SessionEndCallback) {
        *self.hook.lock().unwrap() = Some(hook);
    }
}

#[derive(Default)]
struct RecordingGit {
    calls: Mutex<Vec<Vec<String>>>,
}

impl RecordingGit {
    fn has_call(&self, words: &[&str]) -> bool {
        self.calls
            .lock()
            .unwrap()
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

impl ProcessRunner for RecordingGit {
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

struct Daemon {
    _workspace: TempWorkspace,
    vault: PathBuf,
    skill_root: PathBuf,
    project: Project,
    marks: PrMarks,
    comment_state: PrCommentState,
    bus: Arc<Bus>,
    git: Arc<RecordingGit>,
    clock: FixedClock,
}

fn daemon(tag: &str) -> Daemon {
    let workspace = TempWorkspace::new(tag);
    let root = workspace.path().to_path_buf();
    let vault = root.join("vault");
    let skill_root = root.join("skill");
    let project_dir = root.join(REPO_DIR);
    let state_dir = root.join("state");
    let recipe_dir = skill_root.join(RECIPE_RELATIVE_DIR);
    for dir in [&vault, &project_dir, &state_dir, &recipe_dir] {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(recipe_dir.join(RECIPE_FILE_NAME), RECIPE_STUB).unwrap();
    Daemon {
        vault,
        skill_root,
        project: Project {
            id: PROJECT_ID.to_string(),
            name: REPO_DIR.to_string(),
            path: project_dir.display().to_string(),
            runs_count: 0,
        },
        marks: PrMarks::new(&state_dir),
        comment_state: PrCommentState::new(&state_dir),
        bus: Arc::new(Bus::new()),
        git: Arc::new(RecordingGit::default()),
        clock: FixedClock::from_rfc3339(NOW),
        _workspace: workspace,
    }
}

fn pull(author: &str, state: PrState, branch: &str) -> PrFixPull {
    PrFixPull {
        author: Some(author.to_string()),
        viewer: Some(VIEWER.to_string()),
        state,
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

fn post(
    daemon: &Daemon,
    gh: &Arc<FakeGh>,
    sessions: &FakeSessions,
    body: PrFixRequest,
) -> Result<Value, PrRouteError> {
    let ports = PrFixPorts {
        github: Arc::new(GhPort(gh.clone())),
        run_is_live: Arc::new(|run_id| run_id == HOLDER_RUN_ID),
        host: sessions,
        runner: daemon.git.clone(),
        sleeper: Arc::new(|_| {}),
        bus: daemon.bus.clone(),
        clock: &daemon.clock,
        marks: &daemon.marks,
        comment_state: &daemon.comment_state,
        vault_root: &daemon.vault,
        skill_root: &daemon.skill_root,
        project: &daemon.project,
    };
    fix_pull_request(&ports, body)
}

fn events_of(daemon: &Daemon, phase: PrFixPhase) -> Vec<PrFixSessionEvent> {
    daemon
        .bus
        .recent(EVENT_LIMIT, 0)
        .into_iter()
        .filter(|event| event[EVENT_TYPE_KEY] == PR_FIX_EVENT)
        .map(|event| {
            serde_json::from_value::<PrFixSessionEvent>(event[EVENT_DATA_KEY].clone()).unwrap()
        })
        .filter(|event| event.phase == phase)
        .collect()
}

fn ticket_worktree(daemon: &Daemon) {
    let path = worktrees::worktree_path(&daemon.vault, REPO_DIR, TICKET_KEY);
    std::fs::create_dir_all(path).unwrap();
}

#[test]
fn two_of_four_threads_reach_the_session_and_get_replies() {
    let daemon = daemon("gw-pr-fix-two-of-four");
    let gh = FakeGh::new(pull(VIEWER, PrState::Open, OTHER_BRANCH));
    let sessions = FakeSessions::new(&gh, None);

    let reply = post(&daemon, &gh, &sessions, request(&[THREAD_B, THREAD_D])).unwrap();

    assert_eq!(
        reply["threads"],
        serde_json::to_value([THREAD_B, THREAD_D]).unwrap()
    );
    let prompt = sessions.prompt();
    assert!(prompt.contains(THREAD_B) && prompt.contains(THREAD_D));
    assert!(!prompt.contains(THREAD_A) && !prompt.contains(THREAD_C));
    assert_eq!(events_of(&daemon, PrFixPhase::Started).len(), 1);

    sessions.finish_with_summary();

    assert_eq!(
        gh.replied(),
        vec![THREAD_B.to_string(), THREAD_D.to_string()]
    );
    let ended = events_of(&daemon, PrFixPhase::Ended);
    assert_eq!(ended.len(), 1);
    assert_eq!(ended[0].pushed_sha.as_deref(), Some(PUSHED_SHA));
    assert_eq!(
        ended[0].fixed,
        Some(vec![THREAD_B.to_string(), THREAD_D.to_string()])
    );
    assert_eq!(ended[0].not_relevant, Some(Vec::new()));
    assert_eq!(ended[0].reply_check, PrFixReplyCheck::AllAnswered);
    assert_eq!(ended[0].unanswered, None);
    assert!(daemon.git.has_call(&[WORKTREE_ARG, REMOVE_ARG]));
}

#[test]
fn a_pull_request_of_someone_else_is_forbidden_and_starts_nothing() {
    let daemon = daemon("gw-pr-fix-not-mine");
    let gh = FakeGh::new(pull(OTHER_AUTHOR, PrState::Open, OTHER_BRANCH));
    let sessions = FakeSessions::new(&gh, None);

    let error = post(&daemon, &gh, &sessions, request(&[THREAD_A])).unwrap_err();

    assert_eq!(error.status(), STATUS_FORBIDDEN);
    assert_eq!(error.reason, Some(PrRefusalReason::NotMine));
    assert_eq!(error.body()[REASON_KEY], NOT_MINE_REASON);
    assert_eq!(sessions.started_count(), 0);
    assert!(events_of(&daemon, PrFixPhase::Started).is_empty());
}

#[test]
fn a_merged_pull_request_is_a_conflict() {
    let daemon = daemon("gw-pr-fix-merged");
    let gh = FakeGh::new(pull(VIEWER, PrState::Merged, OTHER_BRANCH));
    let sessions = FakeSessions::new(&gh, None);

    let error = post(&daemon, &gh, &sessions, request(&[THREAD_A])).unwrap_err();

    assert_eq!(error.status(), STATUS_CONFLICT);
    assert_eq!(sessions.started_count(), 0);
}

#[test]
fn a_workspace_held_by_a_workflow_stage_is_a_conflict_naming_the_stage() {
    let daemon = daemon("gw-pr-fix-busy");
    ticket_worktree(&daemon);
    let gh = FakeGh::new(pull(VIEWER, PrState::Open, TICKET_BRANCH));
    let sessions = FakeSessions::new(&gh, Some(STAGE_HOLDER));

    let error = post(&daemon, &gh, &sessions, request(&[THREAD_A])).unwrap_err();

    assert_eq!(error.status(), STATUS_CONFLICT);
    assert!(error.body()[ERROR_KEY]
        .as_str()
        .unwrap()
        .contains(STAGE_NAME));
    assert_eq!(sessions.started_count(), 0);
}

#[test]
fn a_session_failing_before_the_push_keeps_the_worktree_and_the_threads() {
    let daemon = daemon("gw-pr-fix-failure");
    let gh = FakeGh::new(pull(VIEWER, PrState::Open, OTHER_BRANCH));
    let sessions = FakeSessions::new(&gh, None);
    post(&daemon, &gh, &sessions, request(&[THREAD_A, THREAD_C])).unwrap();
    let expected = worktrees::pr_worktree_path(&daemon.vault, REPO_DIR, NUMBER);

    sessions.end(SessionOutcome {
        succeeded: false,
        error: Some(FAILURE_TEXT.to_string()),
        transcript_tail: String::new(),
    });

    let ended = events_of(&daemon, PrFixPhase::Ended);
    assert_eq!(ended.len(), 1);
    assert_eq!(ended[0].error.as_deref(), Some(FAILURE_TEXT));
    assert_eq!(ended[0].worktree_kept, Some(expected.display().to_string()));
    assert!(gh.replied().is_empty());
    assert!(!daemon.git.has_call(&[WORKTREE_ARG, REMOVE_ARG]));
}

/// A stage holding the branch through the guard and the vault file is named in the 409, and freed on release.
#[test]
fn a_branch_taken_through_the_vault_holder_names_the_stage_until_released() {
    let daemon = daemon("gw-pr-fix-vault-holder");
    let gh = FakeGh::new(pull(VIEWER, PrState::Open, OTHER_BRANCH));
    let sessions = FakeSessions::new(&gh, None);
    let lock: Arc<dyn WorkspaceLock> = Arc::new(LocalLock::default());
    let holder = PrWorkspaceHolder {
        stage: STAGE_NAME.to_string(),
        run_id: Some(HOLDER_RUN_ID.to_string()),
        session_id: None,
        acquired_at: HOLDER_ACQUIRED_AT.to_string(),
    };
    pr_workspace_holder::write(&daemon.vault, REPO_DIR, NUMBER, &holder).unwrap();
    let vault = daemon.vault.clone();
    let guard = WorkspaceGuard::acquire_for_stage(&lock, LOCK_KEY, STAGE_NAME)
        .unwrap()
        .on_release(move || pr_workspace_holder::remove(&vault, REPO_DIR, NUMBER));

    let error = post(&daemon, &gh, &sessions, request(&[THREAD_A])).unwrap_err();

    assert_eq!(error.status(), STATUS_CONFLICT);
    let message = error.body()[ERROR_KEY].as_str().unwrap().to_string();
    assert!(
        message.contains(STAGE_NAME) && message.contains(HOLDER_RUN_ID),
        "{message}"
    );
    assert_eq!(sessions.started_count(), 0);

    drop(guard);

    assert!(pr_workspace_holder::read(&daemon.vault, REPO_DIR, NUMBER).is_none());
    post(&daemon, &gh, &sessions, request(&[THREAD_A])).unwrap();
    assert_eq!(sessions.started_count(), 1);
}
