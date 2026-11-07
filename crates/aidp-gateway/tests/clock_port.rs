use aidp_core::clock::FixedClock;
use aidp_gateway::time::{now_iso, now_iso_with, now_ms_with};

#[test]
fn now_iso_parses_as_rfc3339() {
    let value = now_iso();
    assert!(chrono::DateTime::parse_from_rfc3339(&value).is_ok());
}

#[test]
fn now_iso_with_fixed_clock_is_deterministic() {
    let clock = FixedClock::from_rfc3339("2026-01-01T00:00:00Z");
    let first = now_iso_with(&clock);
    let second = now_iso_with(&clock);
    assert_eq!(first, second);
    assert!(chrono::DateTime::parse_from_rfc3339(&first).is_ok());
}

#[test]
fn now_ms_with_fixed_clock_is_deterministic() {
    let clock = FixedClock::from_rfc3339("2026-01-01T00:00:00Z");
    assert_eq!(now_ms_with(&clock), now_ms_with(&clock));
}
