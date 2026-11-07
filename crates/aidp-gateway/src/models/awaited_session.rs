use crate::models::enums::PermissionMode;
use crate::sessions::SessionRecord;

/// How waiting for one stage session ended.
pub struct AwaitedSession {
    pub record: SessionRecord,
    /// The permission mode the session was launched with.
    pub asked_mode: PermissionMode,
    /// The permission mode claude reported, when it did before the wait ended.
    pub observed_mode: Option<String>,
    /// True when claude ran the session under another mode than the preferred one, so it was stopped.
    pub refused: bool,
}
