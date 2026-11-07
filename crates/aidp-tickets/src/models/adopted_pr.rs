use crate::model::Pr;

/// A pull request found for a ticket before any session ran on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AdoptedPr {
    pub pr: Pr,
    pub branch: String,
}
