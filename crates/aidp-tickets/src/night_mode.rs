//! Window planning (`dispatcher.night`, `dispatcher.stop_at_7d_percent`): heavy runs (opus stages)
//! start in the night window and just after a reset of the 5-hour window, and nothing starts once
//! the 7-day window passed its stop threshold. Pure: the local time and the clock come from the caller.

use crate::budget::{RateSnapshot, Verdict, WindowKind};
use crate::config::NightConfig;

const MINUTES_PER_DAY: u32 = 24 * 60;
const MINUTES_PER_HOUR: u32 = 60;
const FIVE_HOUR_WINDOW_MS: i64 = 5 * 60 * 60 * 1000;
const MINUTE_MS: i64 = 60 * 1000;
const PERCENT: f64 = 100.0;

/// Why a heavy run is held back, or that it may start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NightDecision {
    Allow,
    /// Heavy runs wait for the night window.
    OutsideNight,
    /// In the night window, but the 5-hour window was reset too long ago.
    WaitForReset,
}

impl NightDecision {
    /// The reason shown in the dispatch plan, `None` when the run may start.
    pub fn reason(self) -> Option<&'static str> {
        match self {
            NightDecision::Allow => None,
            NightDecision::OutsideNight => Some("night: heavy run waits for the night window"),
            NightDecision::WaitForReset => Some("night: heavy run waits for the next 5h reset"),
        }
    }
}

/// `HH:MM` as minutes since midnight.
pub fn parse_hhmm(text: &str) -> Option<u32> {
    let (hours, minutes) = text.trim().split_once(':')?;
    let (hours, minutes): (u32, u32) = (hours.parse().ok()?, minutes.parse().ok()?);
    (hours < 24 && minutes < MINUTES_PER_HOUR).then_some(hours * MINUTES_PER_HOUR + minutes)
}

/// Whether `minutes` falls in `[start, end)`; a window that ends before it starts wraps past midnight,
/// and equal bounds cover the whole day.
pub fn in_window(minutes: u32, start: u32, end: u32) -> bool {
    let minutes = minutes % MINUTES_PER_DAY;
    match start.cmp(&end) {
        std::cmp::Ordering::Equal => true,
        std::cmp::Ordering::Less => minutes >= start && minutes < end,
        std::cmp::Ordering::Greater => minutes >= start || minutes < end,
    }
}

/// Whether a heavy run may start now. `five_hour_resets_at_s` is the reset instant of the current
/// 5-hour window (Unix seconds) when a snapshot is known. Unparseable bounds disable the mode.
pub fn heavy_decision(
    cfg: &NightConfig,
    local_minutes: u32,
    five_hour_resets_at_s: Option<i64>,
    now_ms: i64,
) -> NightDecision {
    if !cfg.enabled {
        return NightDecision::Allow;
    }
    let (Some(start), Some(end)) = (parse_hhmm(&cfg.start), parse_hhmm(&cfg.end)) else {
        return NightDecision::Allow;
    };
    if !in_window(local_minutes, start, end) {
        return NightDecision::OutsideNight;
    }
    if !cfg.heavy_after_reset {
        return NightDecision::Allow;
    }
    let Some(resets_at_s) = five_hour_resets_at_s else {
        return NightDecision::Allow;
    };
    let window_start_ms = resets_at_s * 1000 - FIVE_HOUR_WINDOW_MS;
    let since_reset_ms = now_ms - window_start_ms;
    if (0..=i64::from(cfg.heavy_window_minutes) * MINUTE_MS).contains(&since_reset_ms) {
        NightDecision::Allow
    } else {
        NightDecision::WaitForReset
    }
}

/// Turns an `Allow` or `Conservative` verdict into a wait on the 7-day window once its
/// utilization reached `stop_percent` (`85.0` = 85 %). Other verdicts are left as they are.
pub fn stop_at_7d(
    verdict: Verdict,
    snapshot: Option<&RateSnapshot>,
    stop_percent: Option<f64>,
    now_ms: i64,
) -> Verdict {
    let (Some(stop), Some(window)) = (stop_percent, snapshot.and_then(|s| s.seven_day)) else {
        return verdict;
    };
    let live = window.resets_at_s * 1000 > now_ms;
    let reached = window.utilization * PERCENT >= stop;
    let open = matches!(
        verdict,
        Verdict::Allow { .. } | Verdict::Conservative { .. }
    );
    if live && reached && open {
        Verdict::Wait {
            until_ms: window.resets_at_s * 1000 + MINUTE_MS,
            window: WindowKind::SevenDay,
            utilization: window.utilization,
            limit: stop / PERCENT,
        }
    } else {
        verdict
    }
}

#[cfg(test)]
#[path = "../tests/unit/night_mode_tests.rs"]
mod tests;
