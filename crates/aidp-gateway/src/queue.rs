//! Launch queue: at most one session runs per workspace, extra launches wait here.
//! `SessionManager` replays them once the running session ends.

use crate::bus::Bus;
use crate::models::enums::queue_reason::QueueReason;
use crate::store::{read_json, write_json};
use crate::sync::lock;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const QUEUE_FILE: &str = "queue.json";
const SESSION_QUEUED_EVENT: &str = "session.queued";
const SESSION_DEQUEUED_EVENT: &str = "session.dequeued";
const DEQUEUE_REASON_CANCELLED: &str = "cancelled";
pub const READ_ONLY_CEILING: usize = 3;

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct QueuedLaunch {
    pub id: String,
    pub project_id: String,
    pub project_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    pub engine: String,
    #[serde(default)]
    pub priority: i32,
    /// Always `queued`; tells a queue entry apart from a SessionRecord.
    pub status: String,
    pub queued_at: String,
    pub origin: crate::sessions::Origin,
    /// Admission key of the workspace the launch waits for; `None` waits on the project id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_key: Option<String>,
    /// Why the launch waits: `workspace-busy` or `read-only-ceiling`; absent on older entries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<QueueReason>,
    /// Raw start request replayed on dequeue; persisted, never exposed over HTTP.
    pub request: serde_json::Value,
}

impl QueuedLaunch {
    /// The key this launch waits on: its workspace key, else its project id.
    pub fn admission_key(&self) -> &str {
        self.workspace_key.as_deref().unwrap_or(&self.project_id)
    }

    /// The only shape HTTP routes return: the record minus `request`.
    pub fn public(&self) -> Value {
        let mut v = json!({
            "id": self.id,
            "projectId": self.project_id,
            "projectName": self.project_name,
            "engine": self.engine,
            "priority": self.priority,
            "status": self.status,
            "queuedAt": self.queued_at,
            "origin": self.origin,
        });
        let obj = v.as_object_mut().expect("object literal");
        if let Some(t) = &self.template_id {
            obj.insert("templateId".to_string(), json!(t));
        }
        if let Some(t) = &self.target {
            obj.insert("target".to_string(), json!(t));
        }
        if let Some(r) = &self.reason {
            obj.insert("reason".to_string(), json!(r));
        }
        v
    }
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub enum Admission {
    Start,
    Queue,
}

/// One running session per workspace; `running_keys` lists the live sessions' workspace keys.
pub fn admit(workspace_key: &str, running_keys: &[String]) -> Admission {
    if running_keys.iter().any(|key| key == workspace_key) {
        Admission::Queue
    } else {
        Admission::Start
    }
}

/// A live writer: its workspace key and the shared-writer group it joined, if any.
pub type WriterLane = (String, Option<String>);

/// True when every live writer on `workspace_key` carries `group`, a non-empty group.
fn shares_writer_group(workspace_key: &str, group: Option<&str>, writers: &[WriterLane]) -> bool {
    let Some(group) = group.filter(|name| !name.is_empty()) else {
        return false;
    };
    writers
        .iter()
        .filter(|(key, _)| key == workspace_key)
        .all(|(_, writer_group)| writer_group.as_deref() == Some(group))
}

/// Admission for a session that may be read-only (permission mode `plan`).
/// Writers need a free workspace key unless every live writer there shares their group.
pub fn admit_lane(
    workspace_key: &str,
    read_only: bool,
    shared_writer_group: Option<&str>,
    writers: &[WriterLane],
    read_only_keys: &[String],
) -> Admission {
    if !read_only {
        let writer_keys: Vec<String> = writers.iter().map(|(key, _)| key.clone()).collect();
        if admit(workspace_key, &writer_keys) == Admission::Start
            || shares_writer_group(workspace_key, shared_writer_group, writers)
        {
            return Admission::Start;
        }
        return Admission::Queue;
    }
    let live = read_only_keys
        .iter()
        .filter(|key| key.as_str() == workspace_key)
        .count();
    if live < READ_ONLY_CEILING {
        Admission::Start
    } else {
        Admission::Queue
    }
}

/// The wire reason recorded on a queued launch.
pub fn queue_reason(read_only: bool) -> QueueReason {
    if read_only {
        QueueReason::ReadOnlyCeiling
    } else {
        QueueReason::WorkspaceBusy
    }
}

pub struct Queue {
    file: PathBuf,
    list: Mutex<Vec<QueuedLaunch>>,
    bus: Arc<Bus>,
}

impl Queue {
    pub fn load(dir: &Path, bus: Arc<Bus>) -> Self {
        let file = dir.join(QUEUE_FILE);
        let list: Vec<QueuedLaunch> = read_json(&file, Vec::new());
        Queue {
            file,
            list: Mutex::new(list),
            bus,
        }
    }

    fn save(&self, list: &[QueuedLaunch]) {
        if let Err(e) = write_json(&self.file, &list) {
            tracing::warn!(error = %e, "cannot save queue");
        }
    }

    /// Dispatch order: `priority` descending, then `queuedAt` ascending.
    fn sorted(mut list: Vec<QueuedLaunch>) -> Vec<QueuedLaunch> {
        list.sort_by(|a, b| {
            b.priority
                .cmp(&a.priority)
                .then_with(|| a.queued_at.cmp(&b.queued_at))
        });
        list
    }

    pub fn list(&self, project_id: Option<&str>) -> Vec<QueuedLaunch> {
        let l = lock(&self.list).clone();
        let filtered: Vec<QueuedLaunch> = match project_id {
            Some(pid) => l.into_iter().filter(|e| e.project_id == pid).collect(),
            None => l,
        };
        Self::sorted(filtered)
    }

    pub fn depth(&self) -> usize {
        lock(&self.list).len()
    }

    pub fn get(&self, id: &str) -> Option<QueuedLaunch> {
        lock(&self.list).iter().find(|e| e.id == id).cloned()
    }

    pub fn enqueue(&self, entry: QueuedLaunch) -> QueuedLaunch {
        {
            let mut l = lock(&self.list);
            l.push(entry.clone());
            self.save(&l);
        }
        self.bus.emit(SESSION_QUEUED_EVENT, entry.public());
        entry
    }

    /// Sets priority to `1 + max` of its project's entries, making it next out.
    pub fn promote(&self, id: &str) -> Option<QueuedLaunch> {
        let mut l = lock(&self.list);
        let project_id = l.iter().find(|e| e.id == id)?.project_id.clone();
        let max_priority = l
            .iter()
            .filter(|e| e.project_id == project_id)
            .map(|e| e.priority)
            .max()
            .unwrap_or(0);
        let entry = l.iter_mut().find(|e| e.id == id)?;
        entry.priority = max_priority + 1;
        let updated = entry.clone();
        self.save(&l);
        Some(updated)
    }

    pub fn cancel(&self, id: &str) -> Option<QueuedLaunch> {
        let removed = {
            let mut l = lock(&self.list);
            let pos = l.iter().position(|e| e.id == id)?;
            let removed = l.remove(pos);
            self.save(&l);
            removed
        };
        self.bus.emit(
            SESSION_DEQUEUED_EVENT,
            json!({
                "queueId": removed.id,
                "projectId": removed.project_id,
                "reason": DEQUEUE_REASON_CANCELLED,
            }),
        );
        Some(removed)
    }

    /// Removes and returns the first entry waiting on `workspace_key`, in dispatch order.
    /// Emits nothing; the caller emits `session.dequeued`.
    pub fn take_next(&self, workspace_key: &str) -> Option<QueuedLaunch> {
        let mut l = lock(&self.list);
        let sorted = Self::sorted(l.clone());
        let next = sorted
            .into_iter()
            .find(|e| e.admission_key() == workspace_key)?;
        let pos = l.iter().position(|e| e.id == next.id)?;
        let removed = l.remove(pos);
        self.save(&l);
        Some(removed)
    }
}

#[cfg(test)]
#[path = "../tests/unit/queue_tests.rs"]
mod tests;
