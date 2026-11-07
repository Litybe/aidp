use aidp_tickets::models::adopted_pr::AdoptedPr;

/// A pull request ready to be adopted, with the draft flag the Jira write-back needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrCandidate {
    pub adopted: AdoptedPr,
    pub is_draft: bool,
}
