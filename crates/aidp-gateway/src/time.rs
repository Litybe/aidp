use aidp_core::clock::{Clock, SystemClock};

pub fn now_iso() -> String {
    now_iso_with(&SystemClock)
}

/// `now_iso`, but reading the time from `clock` instead of the system clock.
pub fn now_iso_with(clock: &dyn Clock) -> String {
    clock
        .now_utc()
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub fn now_ms() -> i64 {
    now_ms_with(&SystemClock)
}

/// `now_ms`, but reading the time from `clock` instead of the system clock.
pub fn now_ms_with(clock: &dyn Clock) -> i64 {
    clock.now_ms()
}
