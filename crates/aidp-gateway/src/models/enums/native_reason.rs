use std::fmt;

/// Why a native run row was skipped, cancelled or failed without a reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NativeReason {
    DependencyFailed,
    NoItems,
    StaticWhenFalse,
    CompositeWhenFalse,
    Cancelled,
    SkippedByUser,
    InterruptedTwice,
    TaskPanicked,
    TaskFailed,
    WroteOutsideFiles,
    OverlappingWrites,
    BlockedTwice,
    IsolationFailed,
    ApplyConflict,
}

impl NativeReason {
    /// The text persisted on the row.
    pub const fn as_str(self) -> &'static str {
        match self {
            NativeReason::DependencyFailed => "a task it depends on failed",
            NativeReason::NoItems => "fan-out has no items",
            NativeReason::StaticWhenFalse => "its when condition is false at plan time",
            NativeReason::CompositeWhenFalse => "the when condition of its composite is false",
            NativeReason::Cancelled => "run cancelled",
            NativeReason::SkippedByUser => "skipped by a user",
            NativeReason::InterruptedTwice => "interrupted twice",
            NativeReason::TaskPanicked => "the task thread panicked",
            NativeReason::TaskFailed => "the task failed",
            NativeReason::WroteOutsideFiles => "wrote outside its files (restored)",
            NativeReason::OverlappingWrites => {
                "wrote the same paths as a task running at the same time"
            }
            NativeReason::BlockedTwice => "blocked-twice",
            NativeReason::IsolationFailed => "could not prepare an isolated worktree for the task",
            NativeReason::ApplyConflict => "its changes conflict with the work of an earlier task",
        }
    }
}

impl fmt::Display for NativeReason {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}
