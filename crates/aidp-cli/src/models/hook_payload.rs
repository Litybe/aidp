use serde::Deserialize;

/// The fields of the JSON that Claude Code sends a hook on stdin.
#[derive(Debug, Clone, Deserialize)]
pub struct HookPayload {
    pub cwd: String,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub transcript_path: Option<String>,
    #[serde(default)]
    pub agent_type: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}
