use serde::{Deserialize, Serialize};

/// Why a flow moved or blocked a ticket, as journaled and shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FlowTicketReason {
    /// The dispatcher started the flow for the ticket.
    #[serde(rename = "flow-started")]
    Started,
    /// The flow's progress moved the ticket status.
    #[serde(rename = "flow-progress", alias = "ticket-delivery flow")]
    StatusSync,
    /// The flow escalated and blocked the ticket.
    #[serde(rename = "flow escalated")]
    Escalated,
    /// The flow failed and blocked the ticket.
    #[serde(rename = "flow failed")]
    Failed,
    /// The ticket reached its token ceiling and the human refused to continue.
    #[serde(rename = "budget")]
    Budget,
}

impl FlowTicketReason {
    /// The wire string, identical to the serialized form.
    pub const fn as_str(self) -> &'static str {
        match self {
            FlowTicketReason::Started => "flow-started",
            FlowTicketReason::StatusSync => "flow-progress",
            FlowTicketReason::Escalated => "flow escalated",
            FlowTicketReason::Failed => "flow failed",
            FlowTicketReason::Budget => "budget",
        }
    }
}
