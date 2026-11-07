//! Integration test: `aidp_tickets::budget` exercised through its public API
//! only (no `#[cfg(test)]` access to `budget.rs`'s internals). One `#[test]`
//! per rule documented on [`aidp_tickets::budget::verdict`], plus the
//! `BudgetStats` learning half: the 30-sample percentile window, sample
//! rejection across a reset, `reserve_for`'s max-of-mix, its fallback to the
//! config default, and the `load`/`save` round trip.
//!
//! UNIT TRAP (see `budget.rs`'s module doc): `resets_at_s` is Unix
//! **seconds**; `now_ms` and `observed_at_ms` are Unix **milliseconds**.
//! Every fixture below builds a "fresh" window with `resets_at_s * 1000 >
//! now_ms` unless a test is deliberately exercising staleness.

use std::fs;
use std::path::PathBuf;

use aidp_testkit::TempWorkspace;
use aidp_tickets::budget::{
    verdict, BudgetStats, RateSnapshot, Sample, Verdict, Window, WindowKind,
};
use aidp_tickets::config::BudgetConfig;

/// A fixed clock so every test's expectations are computable by hand:
/// 2023-11-14T22:13:20Z, deliberately a round number of milliseconds so
/// dividing by 1000 (to build seconds-based `resets_at_s` fixtures) is exact.
const NOW_MS: i64 = 1_700_000_000_000;

fn now_s() -> i64 {
    NOW_MS / 1000
}

fn win(utilization: f64, resets_at_s: i64) -> Window {
    Window {
        utilization,
        resets_at_s,
    }
}

fn mk_snapshot(
    status: &str,
    is_using_overage: bool,
    five_hour: Option<Window>,
    seven_day: Option<Window>,
    observed_at_ms: i64,
) -> RateSnapshot {
    RateSnapshot {
        status: status.to_string(),
        is_using_overage,
        five_hour,
        seven_day,
        observed_at_ms,
    }
}

/// A (before, after) pair with the given 5h/7d utilizations and a shared
/// reset second per window (so no reset falls between the two snapshots),
/// wrapped as a `Sample` at the given `concurrency`.
fn mk_sample(
    five_before: f64,
    five_after: f64,
    seven_before: f64,
    seven_after: f64,
    five_reset_s: i64,
    seven_reset_s: i64,
    concurrency: f64,
) -> Sample {
    Sample {
        before: mk_snapshot(
            "ok",
            false,
            Some(win(five_before, five_reset_s)),
            Some(win(seven_before, seven_reset_s)),
            0,
        ),
        after: mk_snapshot(
            "ok",
            false,
            Some(win(five_after, five_reset_s)),
            Some(win(seven_after, seven_reset_s)),
            1_000,
        ),
        concurrency,
    }
}

/// Record `n` identical-delta samples for `template`, panicking if any is
/// rejected (every fixture the tests below feed this is well-formed: same
/// reset second across before/after, non-negative delta).
fn record_n(stats: &mut BudgetStats, template: &str, delta_5h: f64, n: usize) {
    for i in 0..n {
        let sample = mk_sample(
            0.0,
            delta_5h,
            0.5,
            0.501,
            now_s() + 3_600,
            now_s() + 90_000,
            1.0,
        );
        assert!(
            stats.record(template, &sample, NOW_MS),
            "sample {i} for {template} must be accepted"
        );
    }
}

fn assert_close(actual: f64, expected: f64, msg: &str) {
    assert!(
        (actual - expected).abs() < 1e-9,
        "{msg}: expected {expected}, got {actual}"
    );
}

/// A nanosecond-suffixed temp dir, the pattern this crate's other tests
/// already use (see `tests/plan_dispatch.rs::temp_dir_with_suffix` and
/// Mirrors the private `BudgetStats::path` layout so a test can read back
/// what `save` wrote without a public accessor for it.
fn budget_stats_path(vault_root: &std::path::Path) -> PathBuf {
    vault_root
        .join("TODO")
        .join(".state")
        .join("budget-stats.json")
}

// ---------------------------------------------------------------------
// verdict()
// ---------------------------------------------------------------------

#[test]
fn no_snapshot_and_no_running_dispatcher_allows_one_launch() {
    let cfg = BudgetConfig::default(); // unknown_policy defaults to "conservative"
    let stats = BudgetStats::default();

    let v = verdict(None, 0, 0, &[], &stats, &cfg, NOW_MS);

    match v {
        Verdict::Conservative { max_launches, .. } => assert_eq!(max_launches, 1),
        other => panic!("expected Conservative with no snapshot, got {other:?}"),
    }
}

#[test]
fn no_snapshot_and_one_running_dispatcher_blocks_second_launch() {
    // Acceptance criterion: "a freshly started daemon starts exactly one
    // session, the second waits for the first snapshot." With no snapshot
    // yet and one dispatcher already running on blind faith, verdict() must
    // NOT hand out a flat `1` again -- it is `1usize.saturating_sub(running)`,
    // so a second launch waits.
    let cfg = BudgetConfig::default();
    let stats = BudgetStats::default();

    let v = verdict(None, 1, 0, &[], &stats, &cfg, NOW_MS);

    match v {
        Verdict::Conservative { max_launches, .. } => assert_eq!(
            max_launches, 0,
            "a second dispatcher session must not launch before the first snapshot arrives"
        ),
        other => panic!("expected Conservative with max_launches: 0, got {other:?}"),
    }
}

#[test]
fn five_hour_window_already_reset_is_conservative_not_fresh() {
    // resets_at_s * 1000 == now_ms: the freshness check is `>`, so an equal
    // (or earlier) reset must be judged stale against the injected clock,
    // not against wall-clock time.
    let five_hour = win(0.5, now_s());
    let seven_day = win(0.1, now_s() + 100_000);
    let snap = mk_snapshot("ok", false, Some(five_hour), Some(seven_day), NOW_MS);
    let cfg = BudgetConfig::default();
    let stats = BudgetStats::default();

    let v = verdict(Some(&snap), 0, 0, &[], &stats, &cfg, NOW_MS);

    assert!(
        matches!(v, Verdict::Conservative { .. }),
        "a 5h window whose reset has already passed the injected `now_ms` must be stale, got {v:?}"
    );
}

#[test]
fn rejected_status_waits_on_five_hour() {
    let five_hour = win(0.5, now_s() + 3_600);
    let seven_day = win(0.1, now_s() + 90_000);
    let snap = mk_snapshot("rejected", false, Some(five_hour), Some(seven_day), NOW_MS);
    let cfg = BudgetConfig::default();
    let stats = BudgetStats::default();

    let v = verdict(Some(&snap), 0, 0, &[], &stats, &cfg, NOW_MS);

    match v {
        Verdict::Wait {
            until_ms,
            window,
            utilization,
            limit,
        } => {
            assert_eq!(window, WindowKind::FiveHour);
            assert_eq!(
                until_ms,
                five_hour.resets_at_s * 1_000,
                "a rejected-status Wait resumes exactly at the 5h reset, no grace period"
            );
            assert_close(utilization, five_hour.utilization, "utilization");
            assert_close(limit, cfg.max_utilization_5h, "limit");
        }
        other => panic!("expected Wait on FiveHour for status: rejected, got {other:?}"),
    }
}

#[test]
fn using_overage_waits_on_five_hour() {
    let five_hour = win(0.5, now_s() + 3_600);
    let seven_day = win(0.1, now_s() + 90_000);
    let snap = mk_snapshot("ok", true, Some(five_hour), Some(seven_day), NOW_MS);
    let cfg = BudgetConfig::default();
    let stats = BudgetStats::default();

    let v = verdict(Some(&snap), 0, 0, &[], &stats, &cfg, NOW_MS);

    match v {
        Verdict::Wait {
            until_ms, window, ..
        } => {
            assert_eq!(window, WindowKind::FiveHour);
            assert_eq!(until_ms, five_hour.resets_at_s * 1_000);
        }
        other => panic!("expected Wait on FiveHour for is_using_overage: true, got {other:?}"),
    }
}

#[test]
fn five_hour_at_075_with_010_reserve_cannot_fit_a_whole_run_waits() {
    // headroom = 0.80 - 0.75 = 0.05, which is less than the 0.10 reserve a
    // single run needs: zero whole runs fit, so this must Wait rather than
    // Allow { max_launches: 0, .. }.
    let five_hour = win(0.75, now_s() + 3_600);
    let seven_day = win(0.10, now_s() + 90_000); // low: must not be the constraint
    let snap = mk_snapshot("ok", false, Some(five_hour), Some(seven_day), NOW_MS);
    let cfg = BudgetConfig::default(); // reserve_per_run_5h: 0.10, max_utilization_5h: 0.80
    let stats = BudgetStats::default();

    let v = verdict(Some(&snap), 0, 0, &[], &stats, &cfg, NOW_MS);

    match v {
        Verdict::Wait {
            until_ms,
            window,
            utilization,
            limit,
        } => {
            assert_eq!(window, WindowKind::FiveHour);
            assert_close(utilization, 0.75, "projected 5h utilization");
            assert_close(limit, 0.80, "5h limit");
            assert_eq!(until_ms, five_hour.resets_at_s * 1_000 + 60_000);
        }
        other => panic!(
            "expected Wait on FiveHour: 0.05 headroom cannot fit a 0.10 reserve, got {other:?}"
        ),
    }
}

#[test]
fn five_hour_at_050_with_010_reserve_allows_three_launches() {
    // headroom = 0.80 - 0.50 = 0.30, floor(0.30 / 0.10) = 3.
    let five_hour = win(0.50, now_s() + 3_600);
    let seven_day = win(0.50, now_s() + 90_000); // headroom 0.40 / 0.02 = 20: not the constraint
    let snap = mk_snapshot("ok", false, Some(five_hour), Some(seven_day), NOW_MS);
    let cfg = BudgetConfig::default();
    let stats = BudgetStats::default();

    let v = verdict(Some(&snap), 0, 0, &[], &stats, &cfg, NOW_MS);

    match v {
        Verdict::Allow {
            max_launches,
            headroom_5h,
            reserve_5h,
            ..
        } => {
            assert_eq!(max_launches, 3);
            assert_close(headroom_5h, 0.30, "headroom_5h");
            assert_close(reserve_5h, 0.10, "reserve_5h");
        }
        other => panic!("expected Allow {{ max_launches: 3, .. }}, got {other:?}"),
    }
}

#[test]
fn seven_day_at_089_with_limit_090_waits_on_seven_day() {
    // 7d headroom = 0.90 - 0.89 = 0.01, floor(0.01 / 0.02) = 0: the 7d window
    // is the tighter constraint even though the 5h window has plenty of room.
    let five_hour = win(0.10, now_s() + 3_600); // headroom 0.70 / 0.10 = 7: plenty
    let seven_day = win(0.89, now_s() + 90_000);
    let snap = mk_snapshot("ok", false, Some(five_hour), Some(seven_day), NOW_MS);
    let cfg = BudgetConfig::default(); // reserve_per_run_7d: 0.02, max_utilization_7d: 0.90
    let stats = BudgetStats::default();

    let v = verdict(Some(&snap), 0, 0, &[], &stats, &cfg, NOW_MS);

    match v {
        Verdict::Wait {
            until_ms,
            window,
            limit,
            ..
        } => {
            assert_eq!(window, WindowKind::SevenDay);
            assert_close(limit, 0.90, "7d limit");
            assert_eq!(until_ms, seven_day.resets_at_s * 1_000 + 60_000);
        }
        other => panic!("expected Wait on SevenDay, got {other:?}"),
    }
}

#[test]
fn unreflected_launches_lower_max_launches_by_exactly_two() {
    let five_hour = win(0.30, now_s() + 3_600);
    let seven_day = win(0.30, now_s() + 90_000); // headroom stays huge even with unreflected added in
    let snap = mk_snapshot("ok", false, Some(five_hour), Some(seven_day), NOW_MS);
    let cfg = BudgetConfig::default();
    let stats = BudgetStats::default();

    let baseline = verdict(Some(&snap), 0, 0, &[], &stats, &cfg, NOW_MS);
    let with_two_unreflected = verdict(Some(&snap), 0, 2, &[], &stats, &cfg, NOW_MS);

    let max_baseline = match baseline {
        Verdict::Allow { max_launches, .. } => max_launches,
        other => panic!("expected Allow at unreflected: 0, got {other:?}"),
    };
    let max_with_two = match with_two_unreflected {
        Verdict::Allow { max_launches, .. } => max_launches,
        other => panic!("expected Allow at unreflected: 2, got {other:?}"),
    };

    assert_eq!(max_baseline, 5, "(0.80 - 0.30) / 0.10 = 5");
    assert_eq!(max_with_two, 3, "(0.80 - (0.30 + 2*0.10)) / 0.10 = 3");
    assert_eq!(
        max_baseline - max_with_two,
        2,
        "two still-unreflected launches must lower max_launches by exactly 2"
    );
}

#[test]
fn five_hour_wait_until_ms_is_reset_plus_sixty_second_grace() {
    // Over the ceiling outright (0.82 > 0.80 max): launches_left returns 0
    // directly (negative room), a distinct path from the "not quite enough
    // headroom" case above, exercising the same +60s offset.
    let five_hour = win(0.82, now_s() + 7_200);
    let seven_day = win(0.10, now_s() + 90_000);
    let snap = mk_snapshot("ok", false, Some(five_hour), Some(seven_day), NOW_MS);
    let cfg = BudgetConfig::default();
    let stats = BudgetStats::default();

    let v = verdict(Some(&snap), 0, 0, &[], &stats, &cfg, NOW_MS);

    match v {
        Verdict::Wait {
            until_ms, window, ..
        } => {
            assert_eq!(window, WindowKind::FiveHour);
            assert_eq!(
                until_ms,
                five_hour.resets_at_s * 1_000 + 60_000,
                "a 5h Wait's until_ms must be the window's reset plus the 60s dispatcher-tick grace period"
            );
        }
        other => panic!("expected Wait on FiveHour, got {other:?}"),
    }
}

#[test]
fn same_snapshot_two_clocks_produce_fresh_and_expired_verdicts() {
    let five_hour = win(0.30, now_s() + 1_800);
    let seven_day = win(0.10, now_s() + 90_000);
    let snap = mk_snapshot("ok", false, Some(five_hour), Some(seven_day), NOW_MS);
    let cfg = BudgetConfig::default();
    let stats = BudgetStats::default();

    let fresh = verdict(Some(&snap), 0, 0, &[], &stats, &cfg, NOW_MS);
    assert!(
        matches!(fresh, Verdict::Allow { .. }),
        "before the 5h reset the snapshot must read as fresh, got {fresh:?}"
    );

    let after_reset_ms = five_hour.resets_at_s * 1_000 + 1;
    let expired = verdict(Some(&snap), 0, 0, &[], &stats, &cfg, after_reset_ms);
    assert!(
        matches!(expired, Verdict::Conservative { .. }),
        "the SAME snapshot, evaluated after its 5h reset, must read as stale, got {expired:?}"
    );
}

// ---------------------------------------------------------------------
// BudgetStats
// ---------------------------------------------------------------------

#[test]
fn percentiles_are_computed_over_at_most_the_last_30_samples() {
    let mut stats = BudgetStats::default();
    let five_reset_s = now_s() + 3_600;
    let seven_reset_s = now_s() + 90_000;
    for i in 1..=35i64 {
        let sample = mk_sample(
            0.0,
            i as f64 * 0.001,
            0.5,
            0.501,
            five_reset_s,
            seven_reset_s,
            1.0,
        );
        assert!(
            stats.record("tmpl", &sample, NOW_MS),
            "sample {i} must be accepted"
        );
    }

    // reserve_for's p75-of-mix, read through a zeroed cfg baseline so the
    // learned p75 passes through untouched, is the only public window onto
    // the computed percentile.
    let cfg = BudgetConfig {
        reserve_per_run_5h: 0.0,
        learn: true,
        ..BudgetConfig::default()
    };
    let (reserve_5h, _) = stats.reserve_for(&["tmpl".to_string()], &cfg);
    assert_close(
        reserve_5h,
        0.028,
        "p75 over the last 30 deltas (i = 6..=35) must be 0.028, NOT 0.027 (p75 over all 35, \
         i = 1..=35) -- proving the window really drops the oldest 5 samples",
    );

    // And directly: the persisted file's sample count and raw delta count
    // must both read 30, not 35.
    let _workspace = TempWorkspace::new("budget-verdict-percentile-cap");
    let root = _workspace.path();
    stats.save(root).expect("save must succeed");
    let text = fs::read_to_string(budget_stats_path(root)).expect("read budget-stats.json");
    let json: serde_json::Value = serde_json::from_str(&text).expect("valid json");
    assert_eq!(json["templates"]["tmpl"]["samples"], 30);
    assert_eq!(
        json["templates"]["tmpl"]["deltas_5h"]
            .as_array()
            .expect("deltas_5h must be an array")
            .len(),
        30
    );
}

#[test]
fn record_rejects_a_sample_whose_five_hour_window_reset_between_snapshots() {
    let mut stats = BudgetStats::default();
    let five_reset_s = now_s() + 3_600;
    let seven_reset_s = now_s() + 90_000;
    let sample = Sample {
        before: mk_snapshot(
            "ok",
            false,
            Some(win(0.5, five_reset_s)),
            Some(win(0.5, seven_reset_s)),
            0,
        ),
        after: mk_snapshot(
            "ok",
            false,
            // The 5h window reset in between: a DIFFERENT resets_at_s.
            Some(win(0.1, five_reset_s + 3_600)),
            Some(win(0.5, seven_reset_s)),
            1_000,
        ),
        concurrency: 1.0,
    };

    let accepted = stats.record("tmpl", &sample, NOW_MS);

    assert!(
        !accepted,
        "a reset between before and after must be rejected"
    );
    assert_eq!(
        stats,
        BudgetStats::default(),
        "a rejected sample must leave the stats byte-for-byte untouched"
    );
}

#[test]
fn reserve_for_takes_the_max_p75_across_the_mix() {
    let mut stats = BudgetStats::default();
    record_n(&mut stats, "small", 0.05, 5);
    record_n(&mut stats, "large", 0.12, 5);
    let cfg = BudgetConfig {
        reserve_per_run_5h: 0.0,
        learn: true,
        ..BudgetConfig::default()
    };

    let (reserve_5h, _) = stats.reserve_for(&["small".to_string(), "large".to_string()], &cfg);

    assert_close(
        reserve_5h,
        0.12,
        "reserve_for must take the larger of the two templates' p75, not the first or an average",
    );
}

#[test]
fn reserve_for_falls_back_to_config_when_learn_is_disabled() {
    let mut stats = BudgetStats::default();
    record_n(&mut stats, "tmpl", 0.20, 5); // would win the max() if learn were enabled
    let cfg = BudgetConfig {
        learn: false,
        reserve_per_run_5h: 0.10,
        reserve_per_run_7d: 0.02,
        ..BudgetConfig::default()
    };

    let (reserve_5h, reserve_7d) = stats.reserve_for(&["tmpl".to_string()], &cfg);

    assert_close(reserve_5h, 0.10, "learn: false must ignore the learned p75");
    assert_close(reserve_7d, 0.02, "learn: false must ignore the learned p75");
}

#[test]
fn reserve_for_falls_back_to_config_when_a_template_has_fewer_than_five_samples() {
    let mut stats = BudgetStats::default();
    record_n(&mut stats, "tmpl", 0.30, 4); // below the 5-sample threshold
    let cfg = BudgetConfig {
        learn: true,
        reserve_per_run_5h: 0.10,
        reserve_per_run_7d: 0.02,
        ..BudgetConfig::default()
    };

    let (reserve_5h, reserve_7d) = stats.reserve_for(&["tmpl".to_string()], &cfg);

    assert_close(
        reserve_5h,
        0.10,
        "4 samples must not qualify: fall back to cfg.reserve_per_run_5h even though 0.30 is larger",
    );
    assert_close(reserve_7d, 0.02, "cfg.reserve_per_run_7d fallback");
}

#[test]
fn load_missing_file_yields_defaults_without_erroring() {
    let _workspace = TempWorkspace::new("budget-verdict-load-missing");
    let root = _workspace.path();

    let stats = BudgetStats::load(root);

    assert_eq!(stats, BudgetStats::default());
}

#[test]
fn save_then_load_round_trips() {
    let _workspace = TempWorkspace::new("budget-verdict-roundtrip");
    let root = _workspace.path();
    let mut stats = BudgetStats::default();
    record_n(&mut stats, "tmpl", 0.08, 6);

    stats.save(root).expect("save must succeed");
    let loaded = BudgetStats::load(root);

    assert_eq!(loaded, stats, "a save followed by a load must be lossless");
}
