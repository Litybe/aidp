use serde::{Deserialize, Serialize};

/// How the host says a session ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentOutcome {
    Completed,
    Failed,
    Cancelled,
    Timeout,
    RateLimited,
}

impl AgentOutcome {
    /// The outcome named by `text`, `None` when unknown.
    pub fn parse(text: &str) -> Option<Self> {
        serde_json::from_value(serde_json::Value::String(text.to_string())).ok()
    }
}
