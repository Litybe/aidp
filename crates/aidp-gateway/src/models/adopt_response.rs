use aidp_tickets::model::Ticket;
use serde::Serialize;

/// Answer of `POST /api/tickets/<key>/adopt`: the adopted ticket and its body.
#[derive(Debug, Serialize)]
pub struct AdoptResponse {
    pub ticket: Ticket,
    pub body: String,
}
