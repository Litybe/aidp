use super::*;
use aidp_agents::{AgentControl, AgentError, ExitReason};
use aidp_core::process_runner::ScriptedProcessRunner;
use aidp_testkit::{EnvGuard, TempWorkspace};
use std::sync::atomic::AtomicUsize;

const RECORD: &str = include_str!("../fixtures/claude_permissions/session.json");
const TEST_NAME: &str = "session-shutdown-retry";
const STATE: &str = "state";
const HOME: &str = "HOME";
const USERPROFILE: &str = "USERPROFILE";
const VAULT: &str = "AIDP_VAULT";
const OFF: &str = "off";
const CANCEL_FAILED: &str = "controlled cancellation failure";
const WAIT_FAILED: &str = "controlled wait failure";

struct FailingControl {
    phase: Arc<AtomicUsize>,
}
impl AgentControl for FailingControl {
    fn cancel(&self) -> Result<(), AgentError> {
        if self.phase.load(Ordering::SeqCst) == 0 {
            return Err(AgentError::Io(std::io::Error::other(CANCEL_FAILED)));
        }
        Ok(())
    }
    fn wait(&self) -> Result<ExitReason, AgentError> {
        if self.phase.load(Ordering::SeqCst) == 1 {
            return Err(AgentError::Io(std::io::Error::other(WAIT_FAILED)));
        }
        Ok(ExitReason::Cancelled)
    }
    fn reply_permission(&self, _: &str, _: bool) -> Result<(), AgentError> {
        Err(AgentError::UnsupportedControl)
    }
}

#[test]
fn native_shutdown_propagates_cancel_and_wait_failure_and_retains_session_for_retry() {
    let workspace = TempWorkspace::new(TEST_NAME);
    let _environment = EnvGuard::with(&[
        (HOME, Some(workspace.path().to_str().unwrap())),
        (USERPROFILE, Some(workspace.path().to_str().unwrap())),
        (VAULT, Some(OFF)),
    ]);
    let state = workspace.child(STATE);
    let bus = Arc::new(Bus::new());
    let projects = Arc::new(Projects::load(&state));
    let manager = SessionManager::load_with_runner(
        &state,
        bus.clone(),
        projects.clone(),
        Arc::new(ScriptedProcessRunner::new()),
    );
    let mut record: SessionRecord = serde_json::from_str(RECORD).unwrap();
    record.cwd = workspace.path().display().to_string();
    let id = record.id.clone();
    lock(&manager.index).insert(id.clone(), record);
    let phase = Arc::new(AtomicUsize::new(0));
    let (_, receiver) = std::sync::mpsc::channel();
    let live = Arc::new(Live {
        stdin: Mutex::new(None),
        handle: Mutex::new(Some(AgentHandle::new(
            receiver,
            Box::new(FailingControl {
                phase: phase.clone(),
            }),
        ))),
        events: Mutex::new(Vec::new()),
        last_turn_error: AtomicBool::new(false),
        workflow_tool_uses: Mutex::new(HashSet::new()),
        workflow_runs: Mutex::new(HashSet::new()),
        workflow_stop: Arc::new(AtomicBool::new(false)),
        permission_requests: Mutex::new(std::collections::VecDeque::new()),
    });
    lock(&manager.live).insert(id.clone(), live.clone());
    let native = crate::native_runs::NativeRunManager::new(
        crate::native_runs::NativeDeps::new(
            manager.clone(),
            projects,
            bus,
            Arc::new(crate::flows::registry::AlwaysAllow),
            Arc::new(crate::flows::lock::LocalLock::default()),
            &state,
            None,
        )
        .with_default_vault(PathBuf::new()),
    );
    for (current_phase, message) in [(0, CANCEL_FAILED), (1, WAIT_FAILED)] {
        phase.store(current_phase, Ordering::SeqCst);
        assert!(native
            .try_shutdown()
            .unwrap_err()
            .to_string()
            .contains(message));
        assert_eq!(manager.get(&id).unwrap().status, SessionStatus::Running);
        assert!(lock(&live.handle).is_some());
        assert!(lock(&manager.live).contains_key(&id));
    }
    phase.store(2, Ordering::SeqCst);
    native.try_shutdown().unwrap();
    assert_eq!(manager.get(&id).unwrap().status, SessionStatus::Orphaned);
    native.try_shutdown().unwrap();
}
