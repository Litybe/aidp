use serde::{Deserialize, Serialize};

/// How tool permissions are granted to the agent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionPolicy {
    #[default]
    Default,
    Stdio,
    Bypass,
    Plan,
    /// Never asks: whatever the provider's own allowlist does not grant is refused at once.
    Headless,
}
