use aidp_core::process_runner::SpawnedChild;
use std::sync::Mutex;

/// One action's child, retained until its exit has been confirmed.
pub struct FlowActionProcess {
    pub(crate) child: Mutex<Box<dyn SpawnedChild>>,
    pub(crate) process_group: bool,
    pub(crate) stop_failure: Mutex<Option<String>>,
}
