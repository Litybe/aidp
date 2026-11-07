use crate::models::enums::diff_side::DiffSide;
use crate::models::finding_location::FindingLocation;
use serde::Deserialize;

/// The fields of a review finding needed to post it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ReviewFinding {
    pub id: String,
    #[serde(default)]
    pub severity: Option<String>,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub location: Option<FindingLocation>,
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default)]
    pub line: Option<u32>,
}

impl ReviewFinding {
    /// The explicit location, else the plain `file` and `line` on the right side.
    pub fn anchor(&self) -> Option<FindingLocation> {
        if let Some(location) = &self.location {
            return Some(location.clone());
        }
        Some(FindingLocation {
            path: self.file.clone()?,
            line: self.line?,
            side: DiffSide::Right,
            start_line: None,
        })
    }
}
