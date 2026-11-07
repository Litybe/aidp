use crate::models::squad_pr_row::SquadPrRow;
use aidp_integrations::models::squad_degraded::SquadDegraded;
use serde::Serialize;
use std::collections::BTreeMap;

/// Body of `GET /api/prs`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SquadPrsResponse {
    pub prs: Vec<SquadPrRow>,
    pub lanes: BTreeMap<String, usize>,
    pub fetched_at: String,
    pub degraded: Vec<SquadDegraded>,
}
