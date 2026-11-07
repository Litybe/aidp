/// Fragment naming an illegal status transition, shared by every refusal
/// `model::guard` builds.
pub const NOT_A_LEGAL_TRANSITION: &str = "not a legal transition";

/// Fragment naming the human `--force` rule refusal on a transition leaving
/// `in-progress`.
pub const FORCE_REQUIRED: &str =
    "a human-initiated transition leaving `in-progress` is refused unless --force is passed";

/// Builds the message for an illegal `from -> to` transition, naming the
/// legal targets from `from`.
pub fn illegal_transition(from: &str, to: &str, allowed: &str) -> String {
    format!("cannot transition `{from}` -> `{to}`: {NOT_A_LEGAL_TRANSITION}; from `{from}` the allowed targets are: {allowed}")
}

/// Builds the message for a human-initiated transition refused because it
/// leaves `in-progress` without `--force`.
pub fn force_required(from: &str, to: &str, actor: &str) -> String {
    format!("cannot transition `{from}` -> `{to}` as `{actor}`: {FORCE_REQUIRED}")
}

/// Builds the "ticket not found under <root>" message `TicketStore::get`
/// and `TicketStore::move_to_service` both raise.
pub fn ticket_not_found_under(key: &str, root: &str) -> String {
    format!("ticket `{key}` not found under {root}")
}
