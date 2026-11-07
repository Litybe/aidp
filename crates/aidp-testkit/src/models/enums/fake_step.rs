use std::time::Duration;

use aidp_agents::AgentEvent;

/// One scripted action of a fake agent.
#[derive(Debug, Clone)]
pub enum FakeStep {
    Emit(AgentEvent),
    WriteFile { path: String, contents: String },
    Delay(Duration),
}
