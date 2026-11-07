use aidp_integrations::models::pr_mark::PrMark;
use serde::Serialize;

/// Reply of the marks route: the pull request and its stored mark.
#[derive(Debug, Serialize)]
pub struct PrMarkResponse {
    pub repo: String,
    pub number: u64,
    pub mark: PrMark,
}
