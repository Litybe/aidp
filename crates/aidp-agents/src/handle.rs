use crate::models::enums::agent_error::AgentError;
use crate::models::enums::exit_reason::ExitReason;

/// Provider-side controls of a running agent.
pub trait AgentControl: Send + Sync {
    /// Asks the agent to stop.
    fn cancel(&self) -> Result<(), AgentError>;

    /// Answers a pending permission request.
    fn reply_permission(&self, id: &str, allow: bool) -> Result<(), AgentError>;

    /// Blocks until the agent ends.
    fn wait(&self) -> Result<ExitReason, AgentError>;
}

pub use crate::models::agent_handle::AgentHandle;
