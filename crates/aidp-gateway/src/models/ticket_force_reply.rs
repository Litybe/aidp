use serde::Serialize;

/// Answer of a forced move: the transition made and the flow restarted, if any.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct TicketForceReply {
    pub key: String,
    pub target: String,
    pub from: String,
    pub to: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    pub aborted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flow: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry: Option<String>,
}
