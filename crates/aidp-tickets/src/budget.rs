//! The token-budget guard (lot 10a): decides how many more dispatcher
//! sessions the daemon may launch this tick without pushing the account's
//! rolling 5-hour / 7-day rate-limit windows past the configured ceiling.
//! [`verdict`] is the ONLY place this decision is made.
//!
//! Pure by construction, same discipline as [`crate::plan`] and
//! [`crate::lifecycle`]: no filesystem writes, no network, no clock reads —
//! every function that needs the current time takes `now_ms: i64` from its
//! caller. Never call `Utc::now()` in this module.
//!
//! This module must not depend on `aidp-core` or `aidp-gateway` — see this
//! crate's top-level doc comment for why. The Claude-usage snapshot types the
//! gateway already has ([`crate::budget`] intentionally does NOT reuse
//! `RateLimitInfo` / `RateLimitWindow` or anything named like them) are
//! therefore duplicated here as [`Window`] / [`RateSnapshot`], minimal and
//! owned by this crate. The adapter that builds a [`RateSnapshot`] from the
//! gateway's real usage data lives in `aidp-gateway`, not here.
//!
//! [`BudgetStats`] is the learning half: it keeps, per template, the last 30
//! observed utilization deltas per dispatcher run and derives a p75 reserve
//! from them, persisted at `<vault_root>/TODO/.state/budget-stats.json`.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use chrono::{SecondsFormat, TimeZone, Utc};
use serde::{Deserialize, Serialize};

/// One rate-limit window's live state.
///
/// UNIT TRAP: `resets_at_s` is Unix **seconds** (as the Claude usage API
/// reports it); every other timestamp in this module — `observed_at_ms`,
/// `now_ms`, `until_ms` — is Unix **milliseconds**. Comparing or combining
/// `resets_at_s` with any of those without first multiplying it by 1000
/// silently corrupts the comparison by a factor of 1000. Every call site in
/// this module does `resets_at_s * 1000`; keep that pattern.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Window {
    pub utilization: f64,
    /// Unix seconds — see the struct-level unit trap.
    pub resets_at_s: i64,
}

/// A live snapshot of Claude usage, as read from the gateway's account
/// status at `observed_at_ms`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RateSnapshot {
    pub status: String,
    pub is_using_overage: bool,
    pub five_hour: Option<Window>,
    pub seven_day: Option<Window>,
    /// Unix milliseconds.
    pub observed_at_ms: i64,
}

/// Which rolling window a [`Verdict::Wait`] is blocked on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WindowKind {
    #[serde(rename = "5h")]
    FiveHour,
    #[serde(rename = "7d")]
    SevenDay,
}

/// The budget guard's decision for this tick.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Verdict {
    /// Room for `max_launches` more sessions right now.
    Allow {
        max_launches: usize,
        headroom_5h: f64,
        headroom_7d: f64,
        reserve_5h: f64,
        reserve_7d: f64,
    },
    /// No (or stale) usage data to reason from: launch at most one session
    /// at a time until the first real snapshot arrives.
    Conservative {
        max_launches: usize,
        why: &'static str,
    },
    /// Budget exhausted: wait until `until_ms` before trying again.
    Wait {
        /// Unix milliseconds.
        until_ms: i64,
        window: WindowKind,
        utilization: f64,
        limit: f64,
    },
    /// Refuse every launch this tick, with no instant to resume at.
    Paused { why: String },
}

/// Decide how many more dispatcher sessions may launch this tick, applying
/// these rules in order:
///
/// 1. Unknown utilization — `snap` is `None`, has no `five_hour` window, or
///    that window is already expired (`resets_at_s * 1000 <= now_ms`) —
///    branches on `cfg.unknown_policy`: `"block"` pauses outright;
///    `"conservative"` (and any value this crate does not recognise) allows
///    at most one dispatcher session total, via
///    `1usize.saturating_sub(running_dispatcher)` — NOT a flat `1`, so a
///    daemon that already started one session on blind faith does not start
///    a second every tick, and a fresh daemon starts exactly one; `"allow"`
///    falls through to rule 5 with both utilizations pinned at `0.0` and no
///    window available to wait on.
/// 2. Already rejected or drawing on overage -> [`Verdict::Wait`] on the 5h
///    reset, or [`Verdict::Paused`] if no 5h window is known. Skipped when
///    rule 1 fell through on `unknown_policy = "allow"` (there is no
///    snapshot to read a rejection/overage flag from).
/// 3. Reserves: [`BudgetStats::reserve_for`] (learned p75 per run when
///    enabled and sampled enough, else `cfg.reserve_per_run_5h` /
///    `cfg.reserve_per_run_7d`).
/// 4. Project the utilization each still-`unreflected` launch will add on
///    top of the observed one; a missing `seven_day` window counts as `0.0`.
/// 5. How many more launches fit under each ceiling: `floor((max - u) /
///    reserve)`, clamped at 0; a reserve `<= 0.0` (hand-edited config) is
///    treated as unconstrained rather than divided by. `max_launches` is the
///    smaller of the two window counts.
/// 6. `max_launches == 0` -> [`Verdict::Wait`] on whichever window is the
///    tighter constraint (5h if it is already at/over its ceiling or its own
///    count hit 0, else 7d); falls back to [`Verdict::Paused`] when the
///    window that should be waited on is not actually known, rather than
///    fabricating an instant.
/// 7. Otherwise [`Verdict::Allow`].
#[allow(clippy::too_many_arguments)]
pub fn verdict(
    snap: Option<&RateSnapshot>,
    running_dispatcher: usize,
    unreflected: usize,
    mix: &[String],
    stats: &BudgetStats,
    cfg: &crate::config::BudgetConfig,
    now_ms: i64,
) -> Verdict {
    // Rule 1: unknown utilization -> branch on `unknown_policy`.
    let mut five_hour = snap.and_then(|s| s.five_hour.as_ref());
    let five_hour_fresh = matches!(five_hour, Some(w) if w.resets_at_s * 1000 > now_ms);
    let mut seven_day = snap.and_then(|s| s.seven_day.as_ref());
    let allow_unknown = !five_hour_fresh;

    if !five_hour_fresh {
        match cfg.unknown_policy.as_str() {
            "allow" => {
                five_hour = None;
                seven_day = None;
            }
            "block" => {
                return Verdict::Paused {
                    why: "aucun instantané d'utilisation et unknown_policy=block".to_string(),
                };
            }
            _ => {
                return Verdict::Conservative {
                    max_launches: 1usize.saturating_sub(running_dispatcher),
                    why: "utilisation inconnue jusqu'au premier instantané",
                };
            }
        }
    }

    // Rule 2: already at/over the limit -> wait for the 5h reset.
    if !allow_unknown {
        let s = snap.expect("five_hour_fresh implies a snapshot is present");
        if s.status == "rejected" || s.is_using_overage {
            return match five_hour {
                Some(w) => Verdict::Wait {
                    until_ms: w.resets_at_s * 1000,
                    window: WindowKind::FiveHour,
                    utilization: w.utilization,
                    limit: cfg.max_utilization_5h,
                },
                None => Verdict::Paused {
                    why: "limite atteinte et aucune fenêtre 5 h connue".to_string(),
                },
            };
        }
    }

    // Rule 3: reserve per run, learned from history when enabled.
    let (reserve_5h, reserve_7d) = stats.reserve_for(mix, cfg);

    // Rule 4: project the utilization each still-unreflected launch adds.
    let (u5, u7) = if allow_unknown {
        (0.0, 0.0)
    } else {
        let u5 = five_hour.map(|w| w.utilization).unwrap_or(0.0) + unreflected as f64 * reserve_5h;
        let u7 = seven_day.map(|w| w.utilization).unwrap_or(0.0) + unreflected as f64 * reserve_7d;
        (u5, u7)
    };

    // Rule 5: how many more launches fit under each ceiling.
    let n5 = launches_left(cfg.max_utilization_5h, u5, reserve_5h);
    let n7 = launches_left(cfg.max_utilization_7d, u7, reserve_7d);
    let max_launches = n5.min(n7);

    // Rule 6: no room left -> wait on the tighter-constraining window.
    if max_launches == 0 {
        let wait_on_5h = u5 >= cfg.max_utilization_5h || n5 == 0;
        return if wait_on_5h {
            match five_hour {
                Some(w) => Verdict::Wait {
                    until_ms: w.resets_at_s * 1000 + 60_000,
                    window: WindowKind::FiveHour,
                    utilization: u5,
                    limit: cfg.max_utilization_5h,
                },
                None => Verdict::Paused {
                    why: "budget 5 h épuisé et aucune fenêtre 5 h connue".to_string(),
                },
            }
        } else {
            match seven_day {
                Some(w) => Verdict::Wait {
                    until_ms: w.resets_at_s * 1000 + 60_000,
                    window: WindowKind::SevenDay,
                    utilization: u7,
                    limit: cfg.max_utilization_7d,
                },
                None => Verdict::Paused {
                    why: "budget 7 j épuisé et aucune fenêtre 7 j connue".to_string(),
                },
            }
        };
    }

    // Rule 7: room for at least one more launch.
    Verdict::Allow {
        max_launches,
        headroom_5h: cfg.max_utilization_5h - u5,
        headroom_7d: cfg.max_utilization_7d - u7,
        reserve_5h,
        reserve_7d,
    }
}

/// How many more runs of `reserve` each fit between `projected_utilization`
/// and `max_utilization`. A non-positive `reserve` (a hand-edited config, or
/// a degenerate learned value) is treated as unconstrained rather than
/// divided by.
fn launches_left(max_utilization: f64, projected_utilization: f64, reserve: f64) -> usize {
    if reserve <= 0.0 {
        return usize::MAX;
    }
    let room = (max_utilization - projected_utilization) / reserve;
    if room <= 0.0 {
        0
    } else {
        room.floor() as usize
    }
}

/// One (before, after) pair of snapshots bracketing a dispatcher launch,
/// plus how many sessions were running concurrently at the time — used to
/// normalise the observed jump back down to the cost of a single run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Sample {
    pub before: RateSnapshot,
    pub after: RateSnapshot,
    pub concurrency: f64,
}

/// Per-template learned reserve: how much one dispatcher run typically adds
/// to each window's utilization, derived from the last 30 observed deltas.
/// `#[serde(default)]` on every field so an older or partially-written file
/// still loads.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct TemplateStats {
    samples: usize,
    p50_delta_5h: f64,
    p75_delta_5h: f64,
    p75_delta_7d: f64,
    /// Raw samples backing the summary above, capped at the last 30.
    deltas_5h: Vec<f64>,
    deltas_7d: Vec<f64>,
}

/// The budget guard's learning store: `<vault_root>/TODO/.state/budget-stats.json`.
///
/// Snake_case on the wire deliberately — unlike the rest of this crate's
/// camelCase types, this file is not part of the gateway's camelCase wire
/// contract; it is a private on-disk cache this crate alone reads and
/// writes.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BudgetStats {
    templates: BTreeMap<String, TemplateStats>,
    updated_at: String,
}

impl BudgetStats {
    /// Load `<vault_root>/TODO/.state/budget-stats.json`. A missing, empty,
    /// or corrupt file yields [`BudgetStats::default`] — never an error; a
    /// syntactically invalid file is reported on stderr first, the same
    /// tolerant contract as [`crate::config::load`].
    pub fn load(vault_root: &Path) -> BudgetStats {
        let file = Self::path(vault_root);
        let text = match fs::read_to_string(&file) {
            Ok(t) => t,
            Err(_) => return BudgetStats::default(),
        };
        if text.trim().is_empty() {
            return BudgetStats::default();
        }
        match serde_json::from_str(&text) {
            Ok(stats) => stats,
            Err(e) => {
                tracing::warn!("invalid {}: {e}; falling back to defaults", file.display());
                BudgetStats::default()
            }
        }
    }

    /// Persist to `<vault_root>/TODO/.state/budget-stats.json`, atomically
    /// (via [`crate::store::write_atomic`], which also creates the `.state`
    /// directory if needed).
    pub fn save(&self, vault_root: &Path) -> anyhow::Result<()> {
        let file = Self::path(vault_root);
        let content = serde_json::to_string_pretty(self)?;
        crate::store::write_atomic(&file, &content)
    }

    fn path(vault_root: &Path) -> std::path::PathBuf {
        vault_root
            .join("TODO")
            .join(".state")
            .join("budget-stats.json")
    }

    /// The reserve per run to use for a dispatch mix: for each window, the
    /// max of `cfg.reserve_per_run_*` and the p75 of every template in
    /// `mix` that has at least 5 recorded samples. Falls back to
    /// `cfg.reserve_per_run_5h` / `cfg.reserve_per_run_7d` outright when
    /// `mix` is empty or `cfg.learn` is `false`. The two windows are decided
    /// independently.
    pub fn reserve_for(&self, mix: &[String], cfg: &crate::config::BudgetConfig) -> (f64, f64) {
        if !cfg.learn || mix.is_empty() {
            return (cfg.reserve_per_run_5h, cfg.reserve_per_run_7d);
        }

        let mut reserve_5h = cfg.reserve_per_run_5h;
        let mut reserve_7d = cfg.reserve_per_run_7d;
        for template in mix {
            if let Some(stats) = self.templates.get(template) {
                if stats.samples >= 5 {
                    reserve_5h = reserve_5h.max(stats.p75_delta_5h);
                    reserve_7d = reserve_7d.max(stats.p75_delta_7d);
                }
            }
        }
        (reserve_5h, reserve_7d)
    }

    /// Record one observed sample for `template`, recomputing its summary
    /// statistics. Rejects (returns `false`, mutates nothing) when a reset
    /// fell between the two snapshots (`before.five_hour.resets_at_s !=
    /// after.five_hour.resets_at_s`), when either snapshot is missing its
    /// `five_hour` or `seven_day` window, or when the resulting delta is
    /// negative or not finite — a reset or a missing window makes the delta
    /// meaningless, not just noisy.
    pub fn record(&mut self, template: &str, sample: &Sample, now_ms: i64) -> bool {
        let (Some(before_5h), Some(after_5h)) = (
            sample.before.five_hour.as_ref(),
            sample.after.five_hour.as_ref(),
        ) else {
            return false;
        };
        let (Some(before_7d), Some(after_7d)) = (
            sample.before.seven_day.as_ref(),
            sample.after.seven_day.as_ref(),
        ) else {
            return false;
        };
        if before_5h.resets_at_s != after_5h.resets_at_s {
            return false;
        }

        let concurrency = sample.concurrency.max(1.0);
        let delta_5h = (after_5h.utilization - before_5h.utilization) / concurrency;
        let delta_7d = (after_7d.utilization - before_7d.utilization) / concurrency;
        if !delta_5h.is_finite() || delta_5h < 0.0 || !delta_7d.is_finite() || delta_7d < 0.0 {
            return false;
        }

        let entry = self.templates.entry(template.to_string()).or_default();
        push_capped(&mut entry.deltas_5h, delta_5h, 30);
        push_capped(&mut entry.deltas_7d, delta_7d, 30);
        entry.samples = entry.deltas_5h.len();
        entry.p50_delta_5h = nearest_rank(&entry.deltas_5h, 0.50);
        entry.p75_delta_5h = nearest_rank(&entry.deltas_5h, 0.75);
        entry.p75_delta_7d = nearest_rank(&entry.deltas_7d, 0.75);

        self.updated_at = Utc
            .timestamp_millis_opt(now_ms)
            .single()
            .map(|dt| dt.to_rfc3339_opts(SecondsFormat::Secs, true))
            .unwrap_or_default();

        true
    }
}

/// Push `value` onto `values`, then drop from the front until at most `cap`
/// remain.
fn push_capped(values: &mut Vec<f64>, value: f64, cap: usize) {
    values.push(value);
    while values.len() > cap {
        values.remove(0);
    }
}

/// Nearest-rank percentile `p` (e.g. `0.75`) over `values`. `0.0` for an
/// empty slice. Percentiles over at most 30 `f64` values are cheap enough to
/// compute by sorting a copy — no statistics crate needed.
fn nearest_rank(values: &[f64], p: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = sorted.len();
    let idx = ((p * n as f64).ceil() as usize)
        .saturating_sub(1)
        .min(n - 1);
    sorted[idx]
}
