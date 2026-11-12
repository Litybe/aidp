/// Why the state machine refused an event. The gateway maps every variant
/// to a conflict answer: the event does not fit the run's current state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RejectReason {
    NotStarted,
    AlreadyStarted,
    NotActive,
    WrongState,
    StaleEffect,
    UnknownStep,
    InvalidChoice,
    NotPaused,
    UnknownEntry,
}
