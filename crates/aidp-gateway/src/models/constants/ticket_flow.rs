//! Names the dispatcher uses to hand a ticket to the delivery flow and to find its pull request.

/// Placeholder session id of a flow-launched ticket, which has no session.
pub const FLOW_SESSION_PREFIX: &str = "flow:";
/// Vault folder holding the ticket notes.
pub const TICKETS_DIR: &str = "TODO";
/// Step of the delivery flow whose completion moves the ticket to review.
pub const STEP_READY: &str = "ready";
/// Step of the delivery flow whose completion moves the ticket to done.
pub const STEP_MERGE: &str = "merge";
/// Message of the reply sent when a flow route lookup finds no handler.
pub const FLOW_ROUTE_MISSING: &str = "flow route is not served";
/// Body key carrying a reply's error message.
pub const REPLY_ERROR_KEY: &str = "error";
/// Body key carrying a reply's error code.
pub const REPLY_CODE_KEY: &str = "code";
