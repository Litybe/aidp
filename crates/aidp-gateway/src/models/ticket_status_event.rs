use serde::Serialize;

/// Bus payload of `ticket.status`.
#[derive(Serialize)]
pub struct TicketStatusEvent<'a> {
    pub key: &'a str,
    pub status: &'a str,
}
