use crate::models::constants::plan_slots::WAKE_INTERVAL;
use crate::models::plan_slot_state::PlanSlotState;
use crate::models::task_window::TaskWindow;
use aidp_core::plan::{ScheduledTask, TaskStatus};
use std::collections::HashSet;
use std::sync::{Condvar, Mutex, MutexGuard};

/// Slot counter and per-task state of a plan stage: at most `cap` tasks run at once.
pub struct PlanSlots {
    cap: usize,
    state: Mutex<PlanSlotState>,
    changed: Condvar,
}

impl PlanSlots {
    /// Starts with `settled` tasks already final, such as rows done before a resume.
    pub fn new(cap: usize, settled: Vec<(String, TaskStatus)>) -> Self {
        Self {
            cap: cap.max(1),
            state: Mutex::new(PlanSlotState {
                running: HashSet::new(),
                settled: settled.into_iter().collect(),
                windows: Vec::new(),
                clock: 0,
            }),
            changed: Condvar::new(),
        }
    }

    pub fn cap(&self) -> usize {
        self.cap
    }

    fn lock(&self) -> MutexGuard<'_, PlanSlotState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Blocks until a task may start and returns its index; `None` once nothing more can start.
    pub fn acquire(&self, tasks: &[ScheduledTask], may_launch: &dyn Fn() -> bool) -> Option<usize> {
        let mut guard = self.lock();
        loop {
            Self::skip_blocked(&mut guard, tasks);
            if may_launch() && guard.running.len() < self.cap {
                if let Some(index) = Self::launchable(&guard, tasks) {
                    let id = tasks[index].task.id.clone();
                    guard.clock += 1;
                    let tick = guard.clock;
                    guard.running.insert(id.clone());
                    guard.windows.push(TaskWindow {
                        id,
                        started: tick,
                        ended: None,
                    });
                    return Some(index);
                }
            }
            if guard.running.is_empty() {
                return None;
            }
            guard = self
                .changed
                .wait_timeout(guard, WAKE_INTERVAL)
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
    }

    /// Frees the slot of `id` and records its final status.
    pub fn release(&self, id: &str, status: TaskStatus) {
        let mut guard = self.lock();
        guard.running.remove(id);
        guard.clock += 1;
        let tick = guard.clock;
        if let Some(window) = guard.windows.iter_mut().find(|window| window.id == id) {
            window.ended = Some(tick);
        }
        guard.settled.insert(id.to_string(), status);
        self.changed.notify_all();
    }

    /// Ids marked skipped because a dependency failed.
    pub fn skipped_ids(&self) -> Vec<String> {
        let guard = self.lock();
        let mut ids: Vec<String> = guard
            .settled
            .iter()
            .filter(|(_, status)| **status == TaskStatus::Skipped)
            .map(|(id, _)| id.clone())
            .collect();
        ids.sort();
        ids
    }

    /// Ids whose run window intersected the window of `id`.
    pub fn overlapping_peers(&self, id: &str) -> Vec<String> {
        let guard = self.lock();
        let Some(mine) = guard.windows.iter().find(|window| window.id == id) else {
            return Vec::new();
        };
        let mine_end = mine.ended.unwrap_or(u64::MAX);
        guard
            .windows
            .iter()
            .filter(|other| other.id != id)
            .filter(|other| {
                other.started <= mine_end && mine.started <= other.ended.unwrap_or(u64::MAX)
            })
            .map(|other| other.id.clone())
            .collect()
    }

    fn skip_blocked(state: &mut PlanSlotState, tasks: &[ScheduledTask]) {
        for scheduled in tasks {
            let id = &scheduled.task.id;
            if state.running.contains(id) || state.settled.contains_key(id) {
                continue;
            }
            let blocked = scheduled.deps.iter().any(|dep| {
                state
                    .settled
                    .get(dep)
                    .is_some_and(|status| *status != TaskStatus::Success)
            });
            if blocked {
                state.settled.insert(id.clone(), TaskStatus::Skipped);
            }
        }
    }

    fn launchable(state: &PlanSlotState, tasks: &[ScheduledTask]) -> Option<usize> {
        tasks.iter().position(|scheduled| {
            let id = &scheduled.task.id;
            !state.running.contains(id)
                && !state.settled.contains_key(id)
                && scheduled
                    .deps
                    .iter()
                    .all(|dep| state.settled.get(dep) == Some(&TaskStatus::Success))
                && scheduled
                    .mutex_after
                    .iter()
                    .all(|before| state.settled.contains_key(before))
        })
    }
}
