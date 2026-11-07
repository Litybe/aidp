//! The gateway's single outbound HTTP agent, with explicit timeouts.

use std::sync::OnceLock;
use std::time::Duration;

/// Longest wait to open a TCP connection.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Longest wait for a response read.
pub const READ_TIMEOUT: Duration = Duration::from_secs(30);
/// Longest wait for a request write.
pub const WRITE_TIMEOUT: Duration = Duration::from_secs(30);

static AGENT: OnceLock<ureq::Agent> = OnceLock::new();

/// The shared agent every outbound gateway call goes through (CLI client, webhooks, notifications).
pub fn agent() -> &'static ureq::Agent {
    AGENT.get_or_init(|| {
        ureq::AgentBuilder::new()
            .timeout_connect(CONNECT_TIMEOUT)
            .timeout_read(READ_TIMEOUT)
            .timeout_write(WRITE_TIMEOUT)
            .build()
    })
}
