use serde::Serialize;

/// Answer of `POST /api/memory/model/embed-pending`.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct ModelQueued {
    pub queued: bool,
}
