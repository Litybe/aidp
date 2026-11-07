use std::time::Duration;

pub const PROCESS_POLL: Duration = Duration::from_millis(25);
pub const EXIT_LIMIT: Duration = Duration::from_secs(5);
pub const STOPPED: &str = "flow runtime is shutting down";
pub const EXIT_UNCONFIRMED: &str = "flow action process exit could not be confirmed";
pub const JOIN_FAILED: &str = "flow action worker did not join";
