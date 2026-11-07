//! Constants of the native autonomy contract.

/// Preamble telling an agent it runs autonomously and must journal its choices.
pub const AUTONOMY_PREAMBLE_V1: &str = "You run autonomously: never ask a question. \
When you face a choice, take the recommended option; if none is recommended, take the most prudent \
option and the easiest to undo. Record every such choice in `assumptions[]` with its `choice`, \
its `reason` and the `rejected_alternative`. Never return `blocked` for a decision you can take yourself.";

/// Key of the escalated decisions in a reply's `signals`.
pub const ESCALATIONS_SIGNAL_KEY: &str = "escalations";
