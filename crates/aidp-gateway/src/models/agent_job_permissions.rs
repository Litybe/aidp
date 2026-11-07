use crate::models::enums::permission_intent::PermissionIntent;
use serde::{Deserialize, Serialize};

/// What the session may do and whether anyone can answer it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentJobPermissions {
    pub intent: PermissionIntent,
    pub autonomous: bool,
    pub headless: bool,
    /// How a headless session should read a denial; the host adds it to the session's instructions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guidance: Option<String>,
}
