use serde::{Deserialize, Serialize};

/// The lifecycle state of a session, serialized as its lowercase name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
    /// The session is waiting in the launch queue.
    Queued,
    /// The `claude` process is alive.
    Running,
    /// The process ended with a zero exit code.
    Done,
    /// The process ended with an error.
    Failed,
    /// A user cancelled the session.
    Cancelled,
    /// The daemon restarted while the session was running.
    Orphaned,
}

impl SessionStatus {
    /// The wire value serde writes for this variant.
    pub const fn as_str(self) -> &'static str {
        match self {
            SessionStatus::Queued => "queued",
            SessionStatus::Running => "running",
            SessionStatus::Done => "done",
            SessionStatus::Failed => "failed",
            SessionStatus::Cancelled => "cancelled",
            SessionStatus::Orphaned => "orphaned",
        }
    }

    /// The variant whose wire value is `text`, if any.
    pub fn parse(text: &str) -> Option<Self> {
        [
            SessionStatus::Queued,
            SessionStatus::Running,
            SessionStatus::Done,
            SessionStatus::Failed,
            SessionStatus::Cancelled,
            SessionStatus::Orphaned,
        ]
        .into_iter()
        .find(|status| status.as_str() == text)
    }
}
