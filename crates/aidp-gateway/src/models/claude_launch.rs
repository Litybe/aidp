use aidp_agents::AgentHandle;

use super::stdin_guard::StdinGuard;

/// A started Claude session: its event handle and, when the child has one, its stdin.
pub struct ClaudeLaunch {
    pub handle: AgentHandle,
    pub stdin: Option<StdinGuard>,
}
