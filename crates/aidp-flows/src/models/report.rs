use crate::models::Issue;
use serde::{Deserialize, Serialize};

/// Result of validating a definition.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub errors: Vec<Issue>,
    pub warnings: Vec<Issue>,
}

impl Report {
    /// True when no error was found; warnings do not count.
    pub fn is_valid(&self) -> bool {
        self.errors.is_empty()
    }
}
