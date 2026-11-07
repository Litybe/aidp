//! Process-wide registry of the daemon's resident loops, each held by a [`LoopGuard`].
//! Iterations run through [`guarded`], so a panic ends one iteration, never the loop.

use crate::sync::lock;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

/// The automation scheduler tick loop.
pub const AUTOMATIONS_SCHEDULER: &str = "automations.scheduler";
/// The automation loop consuming internal bus events.
pub const AUTOMATIONS_BUS: &str = "automations.bus";
/// The ticket dispatcher's periodic timer loop.
pub const DISPATCHER_TIMER: &str = "dispatcher.timer";
/// The ticket dispatcher's bus-event loop.
pub const DISPATCHER_BUS: &str = "dispatcher.bus";
/// The run-state watcher loop.
pub const RUNS_WATCHER: &str = "runs.watcher";

/// Snapshot key: live count per loop name.
pub const KEY_RESIDENT_LOOPS: &str = "residentLoops";
/// Snapshot key: sum of every live loop.
pub const KEY_RESIDENT_TOTAL: &str = "residentTotal";
/// Snapshot key: iterations that panicked since the process started.
pub const KEY_LOOP_PANICS: &str = "loopPanics";

const UNKNOWN_PANIC_MESSAGE: &str = "unknown panic payload";

static LOOP_PANICS: AtomicU64 = AtomicU64::new(0);
static LIVE_LOOPS: OnceLock<Mutex<BTreeMap<&'static str, usize>>> = OnceLock::new();

fn live_loops() -> &'static Mutex<BTreeMap<&'static str, usize>> {
    LIVE_LOOPS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

/// Keeps one loop counted as live; dropping it (return or unwind) un-counts it.
#[must_use = "the loop is un-counted as soon as the guard is dropped"]
pub struct LoopGuard {
    name: &'static str,
}

impl Drop for LoopGuard {
    fn drop(&mut self) {
        let mut counts = lock(live_loops());
        let remove_entry = match counts.get_mut(self.name) {
            Some(count) => {
                *count = count.saturating_sub(1);
                *count == 0
            }
            None => false,
        };
        if remove_entry {
            counts.remove(self.name);
        }
    }
}

/// Counts one more live instance of the loop `name` until the guard drops.
pub fn enter_loop(name: &'static str) -> LoopGuard {
    *lock(live_loops()).entry(name).or_insert(0) += 1;
    LoopGuard { name }
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> &str {
    if let Some(text) = payload.downcast_ref::<&str>() {
        text
    } else if let Some(text) = payload.downcast_ref::<String>() {
        text.as_str()
    } else {
        UNKNOWN_PANIC_MESSAGE
    }
}

/// Runs one iteration of loop `name`; a panic is counted, logged and becomes `None`.
pub fn guarded<R>(name: &'static str, body: impl FnOnce() -> R) -> Option<R> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(body)) {
        Ok(result) => Some(result),
        Err(payload) => {
            LOOP_PANICS.fetch_add(1, Ordering::Relaxed);
            let message = panic_message(payload.as_ref());
            tracing::error!(loop_name = name, %message, "resident loop panicked");
            None
        }
    }
}

/// Live loops per name, their total and the panic count, for the health endpoint.
pub fn snapshot() -> Value {
    let counts = lock(live_loops()).clone();
    let total: usize = counts.values().sum();
    json!({
        KEY_RESIDENT_LOOPS: counts,
        KEY_RESIDENT_TOTAL: total,
        KEY_LOOP_PANICS: LOOP_PANICS.load(Ordering::Relaxed),
    })
}

#[cfg(test)]
#[path = "../tests/unit/threads_tests.rs"]
mod tests;
