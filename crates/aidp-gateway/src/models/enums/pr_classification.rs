use crate::models::PrCandidate;

/// How a pull request relates to adoption.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrClassification {
    Adoptable(PrCandidate),
    ClosedUnmerged,
}
