/// What asks for a session extraction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DistillTrigger {
    /// A turn finished; the session goes on.
    Turn,
    /// The session ends or compacts.
    Boundary,
}
