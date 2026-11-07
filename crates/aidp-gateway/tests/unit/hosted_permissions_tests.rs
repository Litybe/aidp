use super::*;
use crate::models::claude_control::ClaudeControl;
use crate::models::constants::claude_permissions;
use crate::models::exit_signal::ExitSignal;
use crate::recording_writer::RecordingWriter;
use aidp_core::process_runner::ScriptedProcessRunner;
use aidp_testkit::TempWorkspace;
use std::sync::Condvar;

const RECORD: &str = include_str!("../fixtures/claude_permissions/session.json");
const REQUEST: &str = include_str!("../fixtures/claude_permissions/request.json");
const REPLY: &str = include_str!("../fixtures/claude_permissions/reply.json");
const EXIT_WAIT_SECONDS: u64 = 5;
const TEST_NAME: &str = "hosted-claude-permissions";
const HOME_ENV: &str = "HOME";
const USERPROFILE_ENV: &str = "USERPROFILE";
const VAULT_ENV: &str = "AIDP_VAULT";
const VAULT_DISABLED: &str = "off";
const STATE_DIR: &str = "state";
const REQUEST_ID: &str = "permission-one";
const NEXT_REQUEST_ID: &str = "permission-two";
const TOOL_NAME: &str = "Bash";
const REQUEST_FIELD: &str = "request";
const INPUT_FIELD: &str = "input";
const ANSWERS_FIELD: &str = "answers";
const ANSWER_FIELD: &str = "answer";
const RESPONSE_FIELD: &str = "response";
const REQUEST_ID_FIELD: &str = "request_id";
const UPDATED_INPUT_FIELD: &str = "updatedInput";
const BEHAVIOR_FIELD: &str = "behavior";
const ALLOW_BEHAVIOR: &str = "allow";
const DENY_BEHAVIOR: &str = "deny";

#[test]
fn hosted_approvals_are_serialized_and_invalid_answers_leave_session_usable() {
    let workspace = TempWorkspace::new(TEST_NAME);
    let _environment = aidp_testkit::EnvGuard::with(&[
        (HOME_ENV, Some(workspace.path().to_str().unwrap())),
        (USERPROFILE_ENV, Some(workspace.path().to_str().unwrap())),
        (VAULT_ENV, Some(VAULT_DISABLED)),
    ]);
    let state = workspace.child(STATE_DIR);
    let projects = Arc::new(crate::projects::Projects::load(&state));
    let manager = SessionManager::load_with_runner(
        &state,
        Arc::new(Bus::new()),
        projects,
        Arc::new(ScriptedProcessRunner::new()),
    );
    let mut record: SessionRecord = serde_json::from_str(RECORD).unwrap();
    record.cwd = workspace.path().to_string_lossy().to_string();
    let session_id = record.id.clone();
    lock(&manager.index).insert(session_id.clone(), record);
    let input = serde_json::from_str::<Value>(REQUEST).unwrap()[REQUEST_FIELD][INPUT_FIELD].clone();
    let writer = RecordingWriter::default();
    let (_, receiver) = std::sync::mpsc::channel();
    let control = ClaudeControl {
        child: Arc::new(Mutex::new(None)),
        stdin: Arc::new(Mutex::new(Some(Box::new(writer.clone())))),
        cancelled: Arc::new(AtomicBool::new(false)),
        exit: Arc::new(ExitSignal {
            reason: Mutex::new(None),
            ended: Condvar::new(),
        }),
        permissions: Arc::new(Mutex::new(BTreeMap::from([
            (REQUEST_ID.to_string(), input.clone()),
            (NEXT_REQUEST_ID.to_string(), input.clone()),
        ]))),
    };
    let live = Arc::new(Live {
        stdin: Mutex::new(Some(Box::new(writer.clone()))),
        handle: Mutex::new(Some(AgentHandle::new(receiver, Box::new(control)))),
        events: Mutex::new(Vec::new()),
        last_turn_error: AtomicBool::new(false),
        workflow_tool_uses: Mutex::new(HashSet::new()),
        workflow_runs: Mutex::new(HashSet::new()),
        workflow_stop: Arc::new(AtomicBool::new(false)),
        permission_requests: Mutex::new(std::collections::VecDeque::new()),
    });
    lock(&manager.live).insert(session_id.clone(), live.clone());
    for request_id in [REQUEST_ID, NEXT_REQUEST_ID] {
        manager.queue_permission_request(
            &session_id,
            &live,
            request_id.to_string(),
            TOOL_NAME.to_string(),
            input.clone(),
        );
    }
    let pending = manager.get(&session_id).unwrap().pending_question.unwrap();
    assert_eq!(pending.permission_request_id.as_deref(), Some(REQUEST_ID));
    assert_eq!(lock(&live.permission_requests).len(), 2);
    let mut invalid: Value = serde_json::from_str(REPLY).unwrap();
    invalid[ANSWERS_FIELD].as_object_mut().unwrap().clear();
    assert_eq!(
        manager
            .reply(&session_id, &invalid)
            .unwrap_err()
            .to_string(),
        AgentError::Config(claude_permissions::INVALID_ANSWER.to_string()).to_string()
    );
    assert!(writer.0.lock().unwrap().is_empty());
    assert_eq!(
        manager
            .get(&session_id)
            .unwrap()
            .pending_question
            .unwrap()
            .permission_request_id
            .as_deref(),
        Some(REQUEST_ID)
    );
    let mut bound_reply: Value = serde_json::from_str(REPLY).unwrap();
    bound_reply[crate::models::constants::question_reply::TOOL_USE_ID] = REQUEST_ID.into();
    manager.reply(&session_id, &bound_reply).unwrap();
    assert_eq!(
        manager
            .get(&session_id)
            .unwrap()
            .pending_question
            .unwrap()
            .permission_request_id
            .as_deref(),
        Some(NEXT_REQUEST_ID)
    );
    assert_eq!(
        manager
            .reply(&session_id, &bound_reply)
            .unwrap_err()
            .to_string(),
        AgentError::Config(crate::models::constants::question_reply::STALE_QUESTION.to_string())
            .to_string()
    );
    assert_eq!(lock(&live.permission_requests).len(), 1);
    assert_eq!(
        manager
            .get(&session_id)
            .unwrap()
            .pending_question
            .unwrap()
            .permission_request_id
            .as_deref(),
        Some(NEXT_REQUEST_ID)
    );
    let mut deny: Value = serde_json::from_str(REPLY).unwrap();
    deny[ANSWERS_FIELD][claude_permissions::QUESTION][ANSWER_FIELD] =
        claude_permissions::DENY.into();
    manager.reply(&session_id, &deny).unwrap();
    assert!(manager.get(&session_id).unwrap().pending_question.is_none());
    assert!(manager.reply(&session_id, &deny).is_err());
    assert!(manager.is_running(&session_id));
    let bytes = writer.0.lock().unwrap();
    let messages: Vec<Value> = std::str::from_utf8(&bytes)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0][RESPONSE_FIELD][REQUEST_ID_FIELD], REQUEST_ID);
    assert_eq!(
        messages[0][RESPONSE_FIELD][RESPONSE_FIELD][BEHAVIOR_FIELD],
        ALLOW_BEHAVIOR
    );
    assert_eq!(
        messages[0][RESPONSE_FIELD][RESPONSE_FIELD][UPDATED_INPUT_FIELD],
        input
    );
    assert_eq!(
        messages[1][RESPONSE_FIELD][REQUEST_ID_FIELD],
        NEXT_REQUEST_ID
    );
    assert_eq!(
        messages[1][RESPONSE_FIELD][RESPONSE_FIELD][BEHAVIOR_FIELD],
        DENY_BEHAVIOR
    );
    assert!(messages[1][RESPONSE_FIELD][RESPONSE_FIELD]
        .get(UPDATED_INPUT_FIELD)
        .is_none());
}

#[test]
fn cancellation_and_shutdown_publish_terminal_status_only_after_agent_exit() {
    for shutting_down in [false, true] {
        let workspace = TempWorkspace::new(TEST_NAME);
        let _environment = aidp_testkit::EnvGuard::with(&[
            (HOME_ENV, Some(workspace.path().to_str().unwrap())),
            (USERPROFILE_ENV, Some(workspace.path().to_str().unwrap())),
            (VAULT_ENV, Some(VAULT_DISABLED)),
        ]);
        let state = workspace.child(STATE_DIR);
        let projects = Arc::new(crate::projects::Projects::load(&state));
        let manager = SessionManager::load_with_runner(
            &state,
            Arc::new(Bus::new()),
            projects,
            Arc::new(ScriptedProcessRunner::new()),
        );
        let mut record: SessionRecord = serde_json::from_str(RECORD).unwrap();
        record.cwd = workspace.path().to_string_lossy().to_string();
        let session_id = record.id.clone();
        lock(&manager.index).insert(session_id.clone(), record);
        let input =
            serde_json::from_str::<Value>(REQUEST).unwrap()[REQUEST_FIELD][INPUT_FIELD].clone();
        let writer = RecordingWriter::default();
        let (_, receiver) = std::sync::mpsc::channel();
        let control = ClaudeControl {
            child: Arc::new(Mutex::new(None)),
            stdin: Arc::new(Mutex::new(Some(Box::new(writer.clone())))),
            cancelled: Arc::new(AtomicBool::new(false)),
            exit: Arc::new(ExitSignal {
                reason: Mutex::new(None),
                ended: Condvar::new(),
            }),
            permissions: Arc::new(Mutex::new(BTreeMap::from([
                (REQUEST_ID.to_string(), input.clone()),
                (NEXT_REQUEST_ID.to_string(), input.clone()),
            ]))),
        };
        let control_cancelled = control.cancelled.clone();
        let control_exit = control.exit.clone();
        let live = Arc::new(Live {
            stdin: Mutex::new(Some(Box::new(writer.clone()))),
            handle: Mutex::new(Some(AgentHandle::new(receiver, Box::new(control)))),
            events: Mutex::new(Vec::new()),
            last_turn_error: AtomicBool::new(false),
            workflow_tool_uses: Mutex::new(HashSet::new()),
            workflow_runs: Mutex::new(HashSet::new()),
            workflow_stop: Arc::new(AtomicBool::new(false)),
            permission_requests: Mutex::new(std::collections::VecDeque::new()),
        });
        let cancelled = control_cancelled;
        let exit = control_exit;
        lock(&manager.live).insert(session_id.clone(), live.clone());

        let manager = Arc::new(manager);
        std::thread::scope(|scope| {
            let worker = scope.spawn(|| {
                if shutting_down {
                    manager.shutdown();
                } else {
                    manager.cancel(&session_id).unwrap();
                }
            });
            let deadline =
                std::time::Instant::now() + std::time::Duration::from_secs(EXIT_WAIT_SECONDS);
            while !cancelled.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
                std::thread::yield_now();
            }
            let cancellation_requested = cancelled.load(Ordering::SeqCst);
            let status_before_exit = manager.get(&session_id).unwrap().status;
            *lock(&exit.reason) = Some(aidp_agents::ExitReason::Cancelled);
            exit.ended.notify_all();
            worker.join().unwrap();
            assert!(cancellation_requested);
            assert_eq!(status_before_exit, SessionStatus::Running);
        });
        assert_eq!(
            manager.get(&session_id).unwrap().status,
            if shutting_down {
                SessionStatus::Orphaned
            } else {
                SessionStatus::Cancelled
            }
        );
    }
}
