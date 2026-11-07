use serde::{Deserialize, Serialize};

const OPEN: &str = "open";
const CLOSED: &str = "closed";

/// The state of a GitHub issue, as `gh` reports it (`OPEN`, `CLOSED`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IssueState {
    #[default]
    Open,
    Closed,
}

impl IssueState {
    /// The wire form, `open` or `closed`.
    pub fn as_str(self) -> &'static str {
        match self {
            IssueState::Open => OPEN,
            IssueState::Closed => CLOSED,
        }
    }

    /// Parses a state ignoring case; anything but `closed` reads as open.
    pub fn from_wire(value: &str) -> Self {
        if value.trim().eq_ignore_ascii_case(CLOSED) {
            IssueState::Closed
        } else {
            IssueState::Open
        }
    }
}
