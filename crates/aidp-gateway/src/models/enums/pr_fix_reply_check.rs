use serde::{Deserialize, Serialize};

/// Outcome of re-reading the chosen threads on GitHub when a fix session ended.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrFixReplyCheck {
    /// No check ran: the event is not an `ended` one.
    #[default]
    NotChecked,
    /// Every chosen thread has a new reply from the PR author.
    AllAnswered,
    /// At least one chosen thread has no new reply from the PR author.
    Unanswered,
    /// GitHub could not be read, so nothing is known.
    Unverified,
}
