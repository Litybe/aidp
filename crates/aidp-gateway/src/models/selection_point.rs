use serde::{Deserialize, Serialize};

/// One free-text review point picked for a `pr-comments` run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SelectionPoint {
    pub id: String,
    pub body: String,
}
