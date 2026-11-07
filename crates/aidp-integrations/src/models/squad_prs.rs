use crate::models::squad_degraded::SquadDegraded;
use crate::models::squad_pr::SquadPr;
use serde::{Deserialize, Serialize};

/// The unfiltered open pull requests of the squad at one instant.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SquadPrs {
    pub fetched_at: String,
    pub me: String,
    pub repos: Vec<String>,
    pub prs: Vec<SquadPr>,
    pub degraded: Vec<SquadDegraded>,
}
