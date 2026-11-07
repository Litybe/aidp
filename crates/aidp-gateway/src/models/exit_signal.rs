use std::sync::{Condvar, Mutex};

use aidp_agents::ExitReason;

/// Lets `wait` block until the pump thread records how the agent ended.
pub(crate) struct ExitSignal {
    pub(crate) reason: Mutex<Option<ExitReason>>,
    pub(crate) ended: Condvar,
}
