//! The one substitutable point of contact with wall-clock time.
//!
//! [`SystemClock`] is the only place in this crate's production code allowed
//! to read [`chrono::Utc::now`] or [`std::time::SystemTime::now`] directly;
//! every other call site takes a `&dyn Clock` instead.

use crate::models::port::SharedPort;

/// A source of the current time.
pub trait Clock: SharedPort {
    /// The current instant, in UTC.
    fn now_utc(&self) -> chrono::DateTime<chrono::Utc>;

    /// The current instant, formatted as RFC 3339 (e.g. `started_at`,
    /// `finished_at` fields in `run-state.json`).
    fn now_rfc3339(&self) -> String;

    /// The current instant, as milliseconds since the Unix epoch.
    fn now_ms(&self) -> i64;
}

/// The real clock: reads [`chrono::Utc::now`] and [`std::time::SystemTime`].
pub struct SystemClock;

impl Clock for SystemClock {
    fn now_utc(&self) -> chrono::DateTime<chrono::Utc> {
        chrono::Utc::now()
    }

    fn now_rfc3339(&self) -> String {
        chrono::Utc::now().to_rfc3339()
    }

    fn now_ms(&self) -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }
}

/// A fixed clock for tests: every call returns the same instant. `pub` (not
/// `cfg(test)`) so other crates' tests can depend on it too.
#[derive(Clone, Copy, Debug)]
pub struct FixedClock(pub chrono::DateTime<chrono::Utc>);

impl FixedClock {
    /// A `FixedClock` from an RFC 3339 string, panicking on a malformed one —
    /// acceptable in test setup code, never in production.
    pub fn from_rfc3339(s: &str) -> Self {
        let parsed = chrono::DateTime::parse_from_rfc3339(s)
            .unwrap_or_else(|e| panic!("invalid RFC3339 timestamp {s:?}: {e}"));
        Self(parsed.with_timezone(&chrono::Utc))
    }
}

impl Clock for FixedClock {
    fn now_utc(&self) -> chrono::DateTime<chrono::Utc> {
        self.0
    }

    fn now_rfc3339(&self) -> String {
        self.0.to_rfc3339()
    }

    fn now_ms(&self) -> i64 {
        self.0.timestamp_millis()
    }
}

#[cfg(test)]
#[path = "../tests/unit/clock_tests.rs"]
mod tests;
