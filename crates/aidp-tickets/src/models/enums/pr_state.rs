use serde::{Deserialize, Serialize};

const OPEN: &str = "open";
const MERGED: &str = "merged";
const CLOSED: &str = "closed";

/// The state of a pull request attached to a ticket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PrState {
    Merged,
    Closed,
    Open,
}

impl PrState {
    /// Parses a PR state, ignoring case; `None` for an unknown value.
    pub fn from_wire(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            OPEN => Some(PrState::Open),
            MERGED => Some(PrState::Merged),
            CLOSED => Some(PrState::Closed),
            _ => None,
        }
    }
}
