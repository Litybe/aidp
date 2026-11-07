use super::flow_action_process::FlowActionProcess;
use aidp_core::process_runner::ProcessRunner;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

/// Tracks only the subprocesses spawned by this flow executor's action jobs.
pub struct FlowActionProcesses {
    pub(crate) runner: Arc<dyn ProcessRunner>,
    pub(crate) stopping: AtomicBool,
    pub(crate) children: Arc<Mutex<Vec<Arc<FlowActionProcess>>>>,
}
