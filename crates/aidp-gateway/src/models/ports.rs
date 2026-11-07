use crate::bus::Bus;
use crate::flows::lock::WorkspaceLock;
use crate::flows::registry::{BudgetGuard, SessionPort};
use aidp_core::clock::Clock;
use aidp_core::id::IdGen;
use aidp_core::process_runner::ProcessRunner;
use std::sync::Arc;
use std::time::Duration;

/// The collaborators the executor reaches the outside world through.
pub struct Ports {
    pub sessions: Arc<dyn SessionPort>,
    pub budget: Arc<dyn BudgetGuard>,
    pub lock: Arc<dyn WorkspaceLock>,
    pub bus: Arc<Bus>,
    pub clock: Arc<dyn Clock>,
    pub ids: Arc<dyn IdGen>,
    pub runner: Arc<dyn ProcessRunner>,
    /// Longest a writing action waits for its workspace before the step retries later.
    pub lock_wait: Duration,
}
