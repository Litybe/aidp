use crate::models::squad_pr_row::SquadPrRow;
use aidp_integrations::models::squad_degraded::SquadDegraded;
use serde::Serialize;

/// Output of `aidp gh squad-prs`.
#[derive(Serialize)]
pub struct SquadPrsEnvelope {
    pub fetched_at: String,
    pub me: String,
    pub repos: Vec<String>,
    pub prs: Vec<SquadPrRow>,
    pub degraded: Vec<SquadDegraded>,
}
