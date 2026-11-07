use serde::{Deserialize, Serialize};

/// The complete prompt of a session, with the fingerprint the host checks.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentJobPrompt {
    pub text: String,
    pub sha256: String,
    pub chars: usize,
}
