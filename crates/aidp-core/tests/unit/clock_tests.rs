use super::*;

#[test]
fn fixed_clock_returns_the_same_instant_every_call() {
    let clock = FixedClock::from_rfc3339("2026-01-01T00:00:00Z");
    assert_eq!(clock.now_rfc3339(), clock.now_rfc3339());
    assert_eq!(clock.now_ms(), clock.now_ms());
    assert_eq!(clock.now_utc(), clock.now_utc());
}

#[test]
fn system_clock_now_ms_is_positive() {
    let clock = SystemClock;
    assert!(clock.now_ms() > 0);
}
