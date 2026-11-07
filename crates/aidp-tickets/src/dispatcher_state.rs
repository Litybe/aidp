//! `<vault>/TODO/.state/dispatcher.json`'s CANONICAL writer shape
//! (F-review__review_maintainability-4): what `aidp-gateway`'s dispatcher
//! actually writes, every tick.
//!
//! This is one of two distinct Rust shapes for that same JSON file, and
//! that duplication is deliberate, not a bug:
//! - **this module** is the writer's always-populated shape — every field
//!   is always serialized (no `skip_serializing_if` anywhere), because the
//!   dispatcher itself constructs a complete value every time it writes.
//! - `crate::index`'s private `DispatcherState` (and its `DispatcherSlots`
//!   / `DispatcherRunning` / `DispatcherLastPlan` / `DispatcherSkip`
//!   satellites) is a SEPARATE, deliberately tolerant reader shape: every
//!   field `#[serde(default)]`, several typed `Option<DateTime<Utc>>` where
//!   this module has a plain `String` — by its own doc comment, resilient
//!   to a partial write, or an older or newer daemon, on purpose.
//!
//! Unifying them into one type would either lose the reader's partial-write
//! tolerance or force the writer's always-present fields into `Option`
//! everywhere. Instead, THIS module is the one canonical home for the
//! writer's shape — `aidp-gateway::dispatcher` imports it rather than
//! redeclaring it — and `index.rs`'s round-trip test
//! (`writer_shape_round_trips_through_the_tolerant_reader`) proves the two
//! shapes still agree on the wire.

use serde::{Deserialize, Serialize};

/// `<vault>/TODO/.state/dispatcher.json`'s exact shape — a contract shared
/// with `_TODO.md`'s renderer and `docs/gateway-api.md`. Every field is
/// always serialized (no `skip_serializing_if` anywhere in this shape) so
/// the key set never varies with the values — see
/// `dispatcher_state_shape_is_stable` in `aidp-gateway::dispatcher`'s tests.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DispatcherState {
    /// `"normal"` | `"waiting"` | `"paused"`. `"waiting"` is lot 10a's.
    pub mode: String,
    pub paused: bool,
    /// `null` until lot 10a's budget guard can set it.
    pub wait_until: Option<String>,
    pub last_tick_at: String,
    pub last_reason: String,
    pub slots: SlotsState,
    pub running: Vec<RunningEntry>,
    pub last_plan: LastPlan,
    /// Lot 10a's budget verdict as of the last tick. `#[serde(default)]` so a
    /// `dispatcher.json` written by lot 9a (which had no such key) still
    /// deserializes on daemon restart instead of silently falling back to
    /// `DispatcherState::default()` and forgetting an in-flight wait.
    #[serde(default)]
    pub budget: BudgetState,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SlotsState {
    pub used: usize,
    pub max: usize,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RunningEntry {
    pub key: String,
    pub session_id: String,
    pub project: String,
    pub started_at: String,
    /// Non-fatal notes recorded when the session was started.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// The warning recorded when a start is refused by the worktree cap, e.g.
/// `worktree-cap: 3 already open (A, B, C)`.
pub fn worktree_cap_warning(max: usize, holders: &[String]) -> String {
    format!(
        "{}: {max} already open ({})",
        crate::plan::SKIP_WORKTREE_CAP,
        holders.join(", ")
    )
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LastPlan {
    pub launched: Vec<String>,
    pub skipped: Vec<SkippedEntry>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SkippedEntry {
    pub key: String,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Lot 10a's flat, always-populated budget verdict — the wire shape shared
/// by the dispatcher that writes it, `index.rs`'s tolerant reader (renders
/// `_TODO.md`'s Budget section), and `GET /api/dispatcher`. Do not rename or
/// reorder a field: all three agree on this exact key set, pinned below by
/// `budget_state_key_set_is_stable`. The rich `budget::Verdict` -> this flat
/// struct conversion lives in `aidp-gateway`, not here — this module stays a
/// plain serializable shape with no logic.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct BudgetState {
    /// `"allow"` | `"conservative"` | `"wait"` | `"paused"`; empty string
    /// on a state that predates a first budget-aware tick.
    pub verdict: String,
    pub max_launches: usize,
    /// Human-readable explanation, e.g. `"fenêtre 5 h à 87 % ≥ 80 %"`. Empty when there is nothing to say.
    pub why: String,
    pub utilization_5h: f64,
    pub utilization_7d: f64,
    pub limit_5h: f64,
    pub limit_7d: f64,
    pub reserve_5h: f64,
    pub reserve_7d: f64,
    pub unreflected: usize,
    /// RFC3339, or `null` when no 5h window is known.
    pub resets_at_5h: Option<String>,
    /// `"5h"` | `"7d"` | `null` — which window a `wait` verdict is waiting on.
    pub wait_window: Option<String>,
    /// RFC3339 of the snapshot this verdict was computed from, or `null`.
    pub observed_at: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pins `BudgetState`'s exact serialized key set — including the
    /// digit-boundary camelCase renames (`utilization_5h` ->
    /// `"utilization5h"`, not `"utilization5H"` or `"utilization_5h"`) that
    /// `index.rs`'s reader and `GET /api/dispatcher` both depend on.
    #[test]
    fn budget_state_key_set_is_stable() {
        let value = serde_json::to_value(BudgetState::default()).unwrap();
        let obj = value
            .as_object()
            .expect("BudgetState serializes to an object");
        let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
        keys.sort_unstable();

        let mut expected = vec![
            "verdict",
            "maxLaunches",
            "why",
            "utilization5h",
            "utilization7d",
            "limit5h",
            "limit7d",
            "reserve5h",
            "reserve7d",
            "unreflected",
            "resetsAt5h",
            "waitWindow",
            "observedAt",
        ];
        expected.sort_unstable();

        assert_eq!(keys, expected);
    }

    /// A lot-9a `dispatcher.json` (no `"budget"` key at all) must still
    /// deserialize, falling back to `BudgetState::default()` for that one
    /// field instead of failing the whole `DispatcherState` and dropping an
    /// in-flight wait.
    #[test]
    fn dispatcher_state_without_budget_key_still_deserializes() {
        let legacy = serde_json::json!({
            "mode": "normal",
            "paused": false,
            "waitUntil": null,
            "lastTickAt": "2026-09-14T00:00:00Z",
            "lastReason": "",
            "slots": { "used": 0, "max": 2 },
            "running": [],
            "lastPlan": { "launched": [], "skipped": [] }
        });

        let state: DispatcherState = serde_json::from_value(legacy).unwrap();
        assert_eq!(state.budget, BudgetState::default());
    }
}
