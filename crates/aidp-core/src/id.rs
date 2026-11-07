//! The one substitutable point of contact with run-id generation.

use crate::clock::Clock;
use crate::models::port::SharedPort;

/// Produces a run id from a [`Clock`].
pub trait IdGen: SharedPort {
    /// A fresh run id, e.g. `20260101-000000-1a2b`.
    fn new_run_id(&self, clock: &dyn Clock) -> String;
}

/// The real generator: reproduces the legacy `run::new_run_id` algorithm —
/// a `YYYYMMDD-HHMMSS` timestamp from the clock, plus a 4-hex-digit suffix
/// from the low 16 bits of the system clock's current nanoseconds.
pub struct SystemIdGen;

impl IdGen for SystemIdGen {
    fn new_run_id(&self, clock: &dyn Clock) -> String {
        let ts = clock.now_utc().format("%Y%m%d-%H%M%S");
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        let rand = format!("{:04x}", nanos & 0xffff);
        format!("{ts}-{rand}")
    }
}

/// A fixed id generator for tests: every call returns the same string.
#[derive(Clone, Debug)]
pub struct FixedIdGen(pub String);

impl IdGen for FixedIdGen {
    fn new_run_id(&self, _clock: &dyn Clock) -> String {
        self.0.clone()
    }
}

#[cfg(test)]
#[path = "../tests/unit/id_tests.rs"]
mod tests;
