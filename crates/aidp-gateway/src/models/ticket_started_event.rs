use serde::Serialize;

/// Bus payload of `ticket.started`.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TicketStartedEvent<'a> {
    pub key: &'a str,
    pub session_id: &'a str,
    pub project: &'a str,
}
