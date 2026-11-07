use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// The milestone a GitHub issue belongs to; the dequeue order serves the nearest due date first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Milestone {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub number: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due_on: Option<DateTime<Utc>>,
}
