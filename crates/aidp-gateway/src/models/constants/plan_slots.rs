use std::time::Duration;

/// Longest wait before a blocked `acquire` re-checks the slot state.
pub const WAKE_INTERVAL: Duration = Duration::from_millis(200);
