/// Answer to a launch request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionStart {
    Started(String),
    Busy,
    /// The session manager queued the launch (and the caller cancelled that queue entry).
    Queued,
}
