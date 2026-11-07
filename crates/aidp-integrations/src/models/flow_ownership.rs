/// Port answering whether an active flow owns a ticket's pull request.
/// The CI autopilot skips the PRs of owned tickets.
pub trait FlowOwnership {
    /// True when an active flow currently owns `ticket_key`.
    fn owns(&self, ticket_key: &str) -> bool;
}
