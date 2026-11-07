/// A session launch: its id and whether it waits in the queue instead of running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartedSession {
    pub id: String,
    pub queued: bool,
}
