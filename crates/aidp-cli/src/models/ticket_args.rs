use aidp_tickets::model::{IssueType, Priority, Source, Status};
use aidp_tickets::Track;

/// Ticket status accepted on the command line, mirrored from [`Status`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum StatusArg {
    /// Waiting to be picked up.
    Todo,
    /// Selected by the dispatcher, not started yet.
    Queued,
    /// A session is working on it.
    InProgress,
    /// A pull request awaits review.
    InReview,
    /// Finished.
    Done,
    /// Stuck until a human or an event unblocks it.
    Blocked,
    /// Abandoned.
    Cancelled,
}

impl From<StatusArg> for Status {
    fn from(arg: StatusArg) -> Self {
        match arg {
            StatusArg::Todo => Self::Todo,
            StatusArg::Queued => Self::Queued,
            StatusArg::InProgress => Self::InProgress,
            StatusArg::InReview => Self::InReview,
            StatusArg::Done => Self::Done,
            StatusArg::Blocked => Self::Blocked,
            StatusArg::Cancelled => Self::Cancelled,
        }
    }
}

/// Ticket priority accepted on the command line, with `P1`..`P5` and `1`..`5` aliases.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum PriorityArg {
    /// Most urgent (`P1`, `1`).
    #[value(aliases = ["p1", "1"])]
    Highest,
    /// Urgent (`P2`, `2`).
    #[value(aliases = ["p2", "2"])]
    High,
    /// Default urgency (`P3`, `3`).
    #[value(aliases = ["p3", "3"])]
    Medium,
    /// Not urgent (`P4`, `4`).
    #[value(aliases = ["p4", "4"])]
    Low,
    /// Least urgent (`P5`, `5`).
    #[value(aliases = ["p5", "5"])]
    Lowest,
}

impl From<PriorityArg> for Priority {
    fn from(arg: PriorityArg) -> Self {
        match arg {
            PriorityArg::Highest => Self::Highest,
            PriorityArg::High => Self::High,
            PriorityArg::Medium => Self::Medium,
            PriorityArg::Low => Self::Low,
            PriorityArg::Lowest => Self::Lowest,
        }
    }
}

/// Ticket origin accepted on the command line, mirrored from [`Source`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum SourceArg {
    /// Mirrored from a GitHub issue.
    Github,
    /// Authored locally.
    Local,
}

impl From<SourceArg> for Source {
    fn from(arg: SourceArg) -> Self {
        match arg {
            SourceArg::Github => Self::Github,
            SourceArg::Local => Self::Local,
        }
    }
}

/// Ticket type accepted on the command line, mirrored from [`IssueType`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum IssueTypeArg {
    /// A user-facing capability.
    Story,
    /// A unit of work.
    Task,
    /// A defect.
    Bug,
    /// A time-boxed investigation.
    Spike,
}

impl From<IssueTypeArg> for IssueType {
    fn from(arg: IssueTypeArg) -> Self {
        match arg {
            IssueTypeArg::Story => Self::Story,
            IssueTypeArg::Task => Self::Task,
            IssueTypeArg::Bug => Self::Bug,
            IssueTypeArg::Spike => Self::Spike,
        }
    }
}

/// Pipeline depth written in a ticket note as `track`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum TrackArg {
    /// Skips the study and the plan gate.
    Quick,
    /// Runs the study and the plan.
    Standard,
    /// Also gates the plan for human review.
    Deep,
    /// Writes the failing test that reproduces the bug first.
    Bug,
}

impl From<TrackArg> for Track {
    fn from(arg: TrackArg) -> Self {
        match arg {
            TrackArg::Quick => Self::Quick,
            TrackArg::Standard => Self::Standard,
            TrackArg::Deep => Self::Deep,
            TrackArg::Bug => Self::Bug,
        }
    }
}

#[cfg(test)]
#[path = "../../tests/unit/ticket_args_tests.rs"]
mod tests;
