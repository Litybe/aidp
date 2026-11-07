use crate::models::review_finding::ReviewFinding;
use serde::Deserialize;

/// A findings file: only the list of findings is read.
#[derive(Debug, Clone, Deserialize)]
pub struct ReviewFindingsFile {
    pub findings: Vec<ReviewFinding>,
}
