use std::sync::mpsc::Receiver;

use crate::handle::AgentControl;
use crate::models::enums::agent_error::AgentError;
use crate::models::enums::agent_event::AgentEvent;
use crate::models::enums::exit_reason::ExitReason;

/// A running agent: its event stream plus its controls.
pub struct AgentHandle {
    events: Receiver<AgentEvent>,
    control: Box<dyn AgentControl>,
}

impl AgentHandle {
    /// Builds a handle from a provider's channel and controls.
    pub fn new(events: Receiver<AgentEvent>, control: Box<dyn AgentControl>) -> Self {
        Self { events, control }
    }

    /// The stream of events, ending with `AgentEvent::Exited`.
    pub fn events(&self) -> &Receiver<AgentEvent> {
        &self.events
    }

    /// Asks the agent to stop.
    pub fn cancel(&self) -> Result<(), AgentError> {
        self.control.cancel()
    }

    /// Answers a pending permission request.
    pub fn reply_permission(&self, id: &str, allow: bool) -> Result<(), AgentError> {
        self.control.reply_permission(id, allow)
    }

    /// Blocks until the agent ends.
    pub fn wait(&self) -> Result<ExitReason, AgentError> {
        self.control.wait()
    }
}
