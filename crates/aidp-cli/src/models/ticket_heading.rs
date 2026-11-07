//! What a pull request title says about its ticket.

use aidp_tickets::model::IssueType;

/// The issue type, the business key and the one-line summary of a ticket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TicketHeading {
    pub issue_type: IssueType,
    pub key: String,
    pub summary: String,
}
