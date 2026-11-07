use super::*;
use crate::bus::Bus;
use aidp_testkit::TempWorkspace;

const WORKSPACE_TAG: &str = "session_lock";
const STATE_DIR: &str = "state";
const LOCK_KEY: &str = "locked-workspace";
const STAGE_NAME: &str = "implement";

fn lock_with_manager(workspace: &TempWorkspace) -> (SessionLock, Arc<SessionManager>) {
    let state_dir = workspace.child(STATE_DIR);
    std::fs::create_dir_all(&state_dir).unwrap();
    let projects = Arc::new(crate::projects::Projects::load(&state_dir));
    let manager = SessionManager::load(&state_dir, Arc::new(Bus::new()), projects);
    (SessionLock::new(manager.clone()), manager)
}

/// A stage acquire refuses a second acquire and names the stage as holder.
#[test]
fn acquire_for_stage_holds_the_workspace_and_names_the_stage() {
    let workspace = TempWorkspace::new(WORKSPACE_TAG);
    let (session_lock, manager) = lock_with_manager(&workspace);

    assert!(session_lock.try_acquire_for_stage(LOCK_KEY, STAGE_NAME));
    assert!(!session_lock.try_acquire(LOCK_KEY));
    assert!(session_lock.is_busy(LOCK_KEY));
    let holder = manager.workspace_holder(LOCK_KEY).unwrap();
    assert!(holder.contains(STAGE_NAME));
}
