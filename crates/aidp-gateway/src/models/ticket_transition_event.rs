use serde::Serialize;

/// Bus payload of `ticket.status` for a manual transition: the documented `from`, `to`,
/// `reason` and `actor`, plus the `status` the dispatcher reads.
#[derive(Serialize)]
pub struct TicketTransitionEvent<'a> {
    pub key: &'a str,
    pub status: &'a str,
    pub from: &'a str,
    pub to: &'a str,
    pub reason: Option<&'a str>,
    pub actor: &'a str,
}
