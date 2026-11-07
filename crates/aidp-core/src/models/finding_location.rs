//! Inline diff position a review finding is anchored to.

use crate::models::enums::diff_side::DiffSide;
use serde::{Deserialize, Serialize};

/// Where a finding lands in the PR diff; mirrors `location` in `findings.schema.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FindingLocation {
    pub path: String,
    pub line: u32,
    pub side: DiffSide,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_line: Option<u32>,
}
