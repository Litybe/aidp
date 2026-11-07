use serde::Serialize;

use super::enums::PermissionBehavior;

/// The allow or deny decision sent back for a tool permission request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PermissionDecision {
    pub behavior: PermissionBehavior,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(rename = "updatedInput", skip_serializing_if = "Option::is_none")]
    pub updated_input: Option<serde_json::Value>,
}
