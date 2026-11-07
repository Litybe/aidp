use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use aidp_agents::{AgentControl, AgentError, ExitReason};

/// Controls of a scripted agent: cancel flag and worker thread.
pub(crate) struct FakeControl {
    pub(crate) cancelled: Arc<AtomicBool>,
    pub(crate) worker: Mutex<Option<JoinHandle<ExitReason>>>,
    pub(crate) outcome: Mutex<Option<ExitReason>>,
}

impl AgentControl for FakeControl {
    fn cancel(&self) -> Result<(), AgentError> {
        self.cancelled.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn reply_permission(&self, _id: &str, _allow: bool) -> Result<(), AgentError> {
        Ok(())
    }

    fn wait(&self) -> Result<ExitReason, AgentError> {
        let mut outcome = self.outcome.lock().expect("outcome lock");
        if let Some(reason) = *outcome {
            return Ok(reason);
        }
        let worker = self.worker.lock().expect("worker lock").take();
        let reason = match worker {
            Some(handle) => handle
                .join()
                .map_err(|_| AgentError::Spawn("fake agent thread panicked".to_string()))?,
            None => ExitReason::Failed,
        };
        *outcome = Some(reason);
        Ok(reason)
    }
}
