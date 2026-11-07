//! Shared HTTP agent with explicit connect, read and write timeouts. The ureq default agent
//! has no read or write timeout, so a server that never answers would block the caller forever.

use std::sync::OnceLock;

use std::time::Duration;

/// Connect, read and write timeouts applied to an agent.
#[derive(Debug, Clone, Copy)]
struct AgentTimeouts {
    connect: Duration,
    read: Duration,
    write: Duration,
}

/// GitHub images: a picture in a comment must not hold a page for long.
const ASSET_TIMEOUTS: AgentTimeouts = AgentTimeouts {
    connect: Duration::from_secs(10),
    read: Duration::from_secs(30),
    write: Duration::from_secs(10),
};

/// Build an agent carrying the three timeouts of `timeouts`.
fn build_agent(timeouts: AgentTimeouts) -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(timeouts.connect)
        .timeout_read(timeouts.read)
        .timeout_write(timeouts.write)
        .build()
}

/// Process-wide agent for GitHub image reads, built on first use. ureq never forwards
/// the `Authorization` header across a redirect.
pub fn asset_agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| build_agent(ASSET_TIMEOUTS))
}
