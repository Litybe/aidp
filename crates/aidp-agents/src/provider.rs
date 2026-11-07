use crate::handle::AgentHandle;
use crate::models::agent_spec::AgentSpec;
use crate::models::capabilities::Capabilities;
use crate::models::enums::agent_error::AgentError;

/// A backend able to run one agent and stream its events.
pub trait AgentProvider: Send + Sync {
    /// Stable provider id, such as `claude`.
    fn id(&self) -> &str;

    /// Features this provider supports.
    fn capabilities(&self) -> Capabilities;

    /// Starts the agent and returns its event stream and controls.
    fn start(&self, spec: AgentSpec) -> Result<AgentHandle, AgentError>;
}
