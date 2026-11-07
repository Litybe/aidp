use super::*;
use crate::budget::Window;

const HOUR_MS: i64 = 3_600_000;

fn night() -> NightConfig {
    NightConfig {
        enabled: true,
        ..NightConfig::default()
    }
}

fn snapshot(seven_day_utilization: f64, resets_at_s: i64) -> RateSnapshot {
    RateSnapshot {
        status: "allowed".to_string(),
        is_using_overage: false,
        five_hour: None,
        seven_day: Some(Window {
            utilization: seven_day_utilization,
            resets_at_s,
        }),
        observed_at_ms: 0,
    }
}

fn allow() -> Verdict {
    Verdict::Allow {
        max_launches: 2,
        headroom_5h: 0.5,
        headroom_7d: 0.5,
        reserve_5h: 0.1,
        reserve_7d: 0.02,
    }
}

#[test]
fn hhmm_parses_valid_times_only() {
    assert_eq!(parse_hhmm("22:30"), Some(22 * 60 + 30));
    assert_eq!(parse_hhmm(" 07:00 "), Some(420));
    assert_eq!(parse_hhmm("24:00"), None);
    assert_eq!(parse_hhmm("7h"), None);
}

#[test]
fn a_window_past_midnight_wraps() {
    let (start, end) = (22 * 60, 7 * 60);
    assert!(in_window(23 * 60, start, end));
    assert!(in_window(3 * 60, start, end));
    assert!(!in_window(7 * 60, start, end));
    assert!(!in_window(12 * 60, start, end));
    assert!(in_window(12 * 60, 9 * 60, 17 * 60));
    assert!(in_window(5, 0, 0));
}

#[test]
fn night_mode_off_allows_everything() {
    assert_eq!(
        heavy_decision(&NightConfig::default(), 12 * 60, None, 0),
        NightDecision::Allow
    );
}

#[test]
fn a_heavy_run_waits_for_the_night() {
    assert_eq!(
        heavy_decision(&night(), 12 * 60, None, 0),
        NightDecision::OutsideNight
    );
    assert!(heavy_decision(&night(), 12 * 60, None, 0)
        .reason()
        .is_some());
}

#[test]
fn at_night_a_heavy_run_starts_just_after_a_reset() {
    let now_ms = 100 * HOUR_MS;
    let reset_started_ms = now_ms - 10 * 60_000;
    let resets_at_s = (reset_started_ms + 5 * HOUR_MS) / 1000;
    assert_eq!(
        heavy_decision(&night(), 23 * 60, Some(resets_at_s), now_ms),
        NightDecision::Allow
    );
}

#[test]
fn at_night_a_heavy_run_waits_when_the_reset_is_old() {
    let now_ms = 100 * HOUR_MS;
    let resets_at_s = (now_ms + 2 * HOUR_MS) / 1000;
    assert_eq!(
        heavy_decision(&night(), 23 * 60, Some(resets_at_s), now_ms),
        NightDecision::WaitForReset
    );
}

#[test]
fn at_night_without_the_reset_rule_or_a_snapshot_a_heavy_run_starts() {
    let mut cfg = night();
    cfg.heavy_after_reset = false;
    assert_eq!(
        heavy_decision(&cfg, 23 * 60, Some(1), 0),
        NightDecision::Allow
    );
    assert_eq!(
        heavy_decision(&night(), 23 * 60, None, 0),
        NightDecision::Allow
    );
}

#[test]
fn bad_bounds_disable_the_mode() {
    let mut cfg = night();
    cfg.start = "soon".to_string();
    assert_eq!(heavy_decision(&cfg, 12 * 60, None, 0), NightDecision::Allow);
}

#[test]
fn the_seven_day_stop_turns_an_allow_into_a_wait() {
    let now_ms = 1_000;
    let verdict = stop_at_7d(allow(), Some(&snapshot(0.86, 10_000)), Some(85.0), now_ms);
    match verdict {
        Verdict::Wait {
            window, until_ms, ..
        } => {
            assert_eq!(window, WindowKind::SevenDay);
            assert_eq!(until_ms, 10_000 * 1000 + 60_000);
        }
        other => panic!("expected a wait, got {other:?}"),
    }
}

#[test]
fn the_seven_day_stop_leaves_other_cases_alone() {
    let now_ms = 1_000;
    assert_eq!(
        stop_at_7d(allow(), Some(&snapshot(0.5, 10_000)), Some(85.0), now_ms),
        allow()
    );
    assert_eq!(
        stop_at_7d(allow(), Some(&snapshot(0.99, 10_000)), None, now_ms),
        allow()
    );
    assert_eq!(stop_at_7d(allow(), None, Some(85.0), now_ms), allow());
    assert_eq!(
        stop_at_7d(allow(), Some(&snapshot(0.99, 0)), Some(85.0), now_ms),
        allow()
    );
}
