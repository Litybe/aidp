use aidp_integrations::models::squad_degraded::SquadDegraded;
use serde::Serialize;

/// Output of `aidp gh squad-repos`.
#[derive(Serialize)]
pub struct SquadReposEnvelope {
    pub repos: Vec<String>,
    pub degraded: Vec<SquadDegraded>,
}
