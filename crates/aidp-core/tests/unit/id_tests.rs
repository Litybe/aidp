use super::*;
use crate::clock::FixedClock;

#[test]
fn fixed_id_gen_ignores_the_clock_and_always_returns_the_same_id() {
    let gen = FixedIdGen("FIXED-0001".to_string());
    let clock = FixedClock::from_rfc3339("2026-01-01T00:00:00Z");
    assert_eq!(gen.new_run_id(&clock), "FIXED-0001");
    assert_eq!(gen.new_run_id(&clock), "FIXED-0001");
}

#[test]
fn system_id_gen_embeds_the_clock_timestamp() {
    let gen = SystemIdGen;
    let clock = FixedClock::from_rfc3339("2026-01-01T00:00:00Z");
    let id = gen.new_run_id(&clock);
    assert!(id.starts_with("20260101-000000-"));
    assert_eq!(id.len(), "20260101-000000-".len() + 4);
}
