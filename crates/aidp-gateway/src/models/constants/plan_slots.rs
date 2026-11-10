use std::time::Duration;

/// Longest wait before a blocked `acquire` re-checks the slot state.
pub const WAKE_INTERVAL: Duration = Duration::from_millis(200);

/// Flow phase of the merge approval: a flow waiting there has finished its work and holds no slot.
pub const MERGE_PHASE: &str = "Merge";
