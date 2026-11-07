use crate::models::enums::diff_side::DiffSide;
use serde::Deserialize;

fn right_side() -> DiffSide {
    DiffSide::Right
}

/// Where a finding points in the diff.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct FindingLocation {
    pub path: String,
    pub line: u32,
    #[serde(default = "right_side")]
    pub side: DiffSide,
    #[serde(default)]
    pub start_line: Option<u32>,
}
