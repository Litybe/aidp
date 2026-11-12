use serde::{Deserialize, Serialize};

/// Body of `POST /api/tickets/:key/force`: the board column to move the ticket to.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TicketForceRequest {
    pub target: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}
