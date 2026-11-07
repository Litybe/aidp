use serde::Serialize;

use super::enums::ControlSubtype;
use super::permission_decision::PermissionDecision;

/// The `response` of a control response line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ControlResponseBody {
    pub subtype: ControlSubtype,
    pub request_id: String,
    pub response: PermissionDecision,
}
