//! Forced moves of a ticket to a board column.

/// Prefix of the journalled reason of a forced move, followed by the target.
pub const FORCED_REASON_PREFIX: &str = "forced:";
/// Separator between the forced reason and the human's own reason.
pub const REASON_SEPARATOR: &str = " ";
/// `autopilot.blocked_reason` of a ticket forced into the blocked column.
pub const FORCED_BLOCKED_REASON: &str = "forced";
/// Actor recorded on the flow abort a forced move makes.
pub const FORCE_ACTOR: &str = "human";
/// Bus event of a ticket status change.
pub const EVENT_TICKET_STATUS: &str = "ticket.status";
/// Refusal of an unknown target.
pub const UNKNOWN_TARGET: &str =
    "target must be ready, review, to-test, testing, merge, blocked or done";
/// Refusal when the flow engine is not attached.
pub const FLOWS_UNAVAILABLE: &str = "the flow engine is not running";
