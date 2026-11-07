//! Daemon resources with a single holder and a FIFO of waiters, held in memory only.
//!
//! The holder is derived from the flow runs: a step with `resource:` that is running holds it,
//! so a restart rebuilds the holders from the persisted runs and never leaves a phantom lock.

use crate::sync::lock;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Mutex;

/// Bus event published on every change of a resource (holder or waiters).
pub const EVENT_LOCK_CHANGED: &str = "unity.lock_changed";

/// Resource key of the editor of project `project_id`: `unity-editor:<project id>`.
pub fn resource_key(resource: &str, project_id: &str) -> String {
    format!("{resource}:{project_id}")
}

/// Who holds a resource.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceHolder {
    pub flow_key: String,
    pub ticket_key: String,
    pub step_id: String,
    pub since: String,
}

/// Answer of [`ResourceManager::acquire`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Acquire {
    Granted,
    Busy,
}

/// State of one resource after a change, the payload of `unity.lock_changed`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    pub resource: String,
    pub held: bool,
    pub holder: Option<ResourceHolder>,
    pub waiting: Vec<String>,
    /// True when the change freed the resource: waiters should be woken.
    #[serde(skip)]
    pub released: bool,
}

#[derive(Default)]
struct Slot {
    holder: Option<ResourceHolder>,
    queue: VecDeque<String>,
    /// Flows whose hold was released by hand: they are not re-claimed while they still run.
    forced: BTreeSet<String>,
}

impl Slot {
    fn change(&self, resource: &str, released: bool) -> Change {
        Change {
            resource: resource.to_string(),
            held: self.holder.is_some(),
            holder: self.holder.clone(),
            waiting: self.queue.iter().cloned().collect(),
            released,
        }
    }

    fn holds(&self, flow_key: &str) -> bool {
        self.holder.as_ref().is_some_and(|h| h.flow_key == flow_key)
    }

    fn may_take(&self, flow_key: &str) -> bool {
        self.holder.is_none() && self.queue.front().is_none_or(|front| front == flow_key)
    }
}

/// Single-holder resources with a FIFO of waiters.
#[derive(Default)]
pub struct ResourceManager {
    slots: Mutex<BTreeMap<String, Slot>>,
}

impl ResourceManager {
    /// Takes `resource` for `holder`, or queues the flow behind the current holder.
    /// Idempotent for the flow that already holds it.
    pub fn acquire(&self, resource: &str, holder: ResourceHolder) -> (Acquire, Option<Change>) {
        let mut slots = lock(&self.slots);
        let slot = slots.entry(resource.to_string()).or_default();
        if slot.holds(&holder.flow_key) {
            return (Acquire::Granted, None);
        }
        if slot.may_take(&holder.flow_key) {
            slot.queue.retain(|flow| flow != &holder.flow_key);
            slot.forced.remove(&holder.flow_key);
            slot.holder = Some(holder);
            return (Acquire::Granted, Some(slot.change(resource, false)));
        }
        if slot.queue.contains(&holder.flow_key) {
            return (Acquire::Busy, None);
        }
        slot.queue.push_back(holder.flow_key);
        (Acquire::Busy, Some(slot.change(resource, false)))
    }

    /// Records a flow that is already running its step as holder, never queueing it.
    /// False when someone else holds the resource or the hold was released by hand.
    pub fn claim(&self, resource: &str, holder: ResourceHolder) -> (bool, Option<Change>) {
        let mut slots = lock(&self.slots);
        let slot = slots.entry(resource.to_string()).or_default();
        if slot.holds(&holder.flow_key) {
            return (true, None);
        }
        if !slot.may_take(&holder.flow_key) || slot.forced.contains(&holder.flow_key) {
            return (false, None);
        }
        slot.queue.retain(|flow| flow != &holder.flow_key);
        slot.holder = Some(holder);
        (true, Some(slot.change(resource, false)))
    }

    /// True when `flow_key` could take the resource right now (free and first in line).
    pub fn can_grant(&self, resource: &str, flow_key: &str) -> bool {
        let slots = lock(&self.slots);
        slots
            .get(resource)
            .is_none_or(|slot| slot.holds(flow_key) || slot.may_take(flow_key))
    }

    /// Aligns the resources with what run `flow_key` now does: it holds `holds` (at most one
    /// resource) and waits for `waiting`. Anything else it held or queued for is dropped.
    pub fn sync(&self, flow_key: &str, holds: Option<&str>, waiting: Option<&str>) -> Vec<Change> {
        let mut slots = lock(&self.slots);
        let mut changes = Vec::new();
        for (resource, slot) in slots.iter_mut() {
            let mut released = false;
            let mut changed = false;
            if holds != Some(resource.as_str()) {
                if slot.holds(flow_key) {
                    slot.holder = None;
                    released = true;
                    changed = true;
                }
                slot.forced.remove(flow_key);
            }
            if waiting != Some(resource.as_str()) && holds != Some(resource.as_str()) {
                let before = slot.queue.len();
                slot.queue.retain(|flow| flow != flow_key);
                changed |= slot.queue.len() != before;
            }
            if changed {
                changes.push(slot.change(resource, released));
            }
        }
        changes
    }

    /// Frees `resource` whoever holds it (operator action). Returns the former holder.
    pub fn force_release(&self, resource: &str) -> Option<(ResourceHolder, Change)> {
        let mut slots = lock(&self.slots);
        let slot = slots.get_mut(resource)?;
        let holder = slot.holder.take()?;
        slot.forced.insert(holder.flow_key.clone());
        let change = slot.change(resource, true);
        Some((holder, change))
    }

    /// Current holder and waiters of `resource`.
    pub fn snapshot(&self, resource: &str) -> (Option<ResourceHolder>, Vec<String>) {
        let slots = lock(&self.slots);
        slots.get(resource).map_or((None, Vec::new()), |slot| {
            (slot.holder.clone(), slot.queue.iter().cloned().collect())
        })
    }
}

#[cfg(test)]
#[path = "../tests/unit/unity_lock_tests.rs"]
mod tests;
