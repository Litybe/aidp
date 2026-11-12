use aidp_tickets::model::{Phase, Status};

/// A board column a ticket can be forced into by hand, skipping the steps before it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForceTarget {
    Ready,
    ToTest,
    Review,
    Testing,
    Merge,
    Blocked,
    Done,
}

impl ForceTarget {
    /// Every target, in board order.
    pub const ALL: [ForceTarget; 7] = [
        ForceTarget::Ready,
        ForceTarget::Review,
        ForceTarget::ToTest,
        ForceTarget::Testing,
        ForceTarget::Merge,
        ForceTarget::Blocked,
        ForceTarget::Done,
    ];

    /// The wire form, also the name of the flow entry a target restarts from.
    pub const fn as_str(self) -> &'static str {
        match self {
            ForceTarget::Ready => "ready",
            ForceTarget::ToTest => "to-test",
            ForceTarget::Review => "review",
            ForceTarget::Testing => "testing",
            ForceTarget::Merge => "merge",
            ForceTarget::Blocked => "blocked",
            ForceTarget::Done => "done",
        }
    }

    /// The target named `text`, ignoring case.
    pub fn parse(text: &str) -> Option<ForceTarget> {
        Self::ALL
            .into_iter()
            .find(|target| target.as_str().eq_ignore_ascii_case(text.trim()))
    }

    /// The ticket status the move writes.
    pub const fn status(self) -> Status {
        match self {
            ForceTarget::Ready => Status::Todo,
            ForceTarget::ToTest
            | ForceTarget::Review
            | ForceTarget::Testing
            | ForceTarget::Merge => Status::InProgress,
            ForceTarget::Blocked => Status::Blocked,
            ForceTarget::Done => Status::Done,
        }
    }

    /// The lane phase the move writes; `None` keeps the ticket's own.
    pub const fn phase(self) -> Option<Phase> {
        match self {
            ForceTarget::Ready | ForceTarget::Review => Some(Phase::Dev),
            ForceTarget::ToTest => Some(Phase::ToTest),
            ForceTarget::Testing | ForceTarget::Merge => Some(Phase::Testing),
            ForceTarget::Blocked | ForceTarget::Done => None,
        }
    }

    /// The `aidp:*` state label the issue of a GitHub mirror gets.
    pub const fn state_label(self) -> &'static str {
        match self {
            ForceTarget::Ready => "ready",
            ForceTarget::ToTest => "to-test",
            ForceTarget::Review => "review",
            ForceTarget::Testing => "testing",
            ForceTarget::Merge => "awaiting-merge",
            ForceTarget::Blocked => "blocked",
            ForceTarget::Done => "done",
        }
    }

    /// True when the move restarts a flow of the route from the entry named after the target.
    pub const fn restarts_flow(self) -> bool {
        matches!(
            self,
            ForceTarget::Review | ForceTarget::Testing | ForceTarget::Merge
        )
    }

    /// True when the test flow of the route is tried before the delivery flow.
    pub const fn test_lane_first(self) -> bool {
        matches!(self, ForceTarget::Testing | ForceTarget::Merge)
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/force_target_tests.rs"]
mod tests;
