/// Answer to a launch request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionStart {
    Started(String),
    Busy,
}
