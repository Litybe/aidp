use crate::models::constants::settings::{MODEL_STAGES, MODEL_TIERS};
use aidp_core::model_policy::ModelPolicy;
use aidp_tickets::models::enums::BudgetRoute;
use serde::Serialize;

/// Body of `GET /api/settings/models`: the policy and the words it may use.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ModelsSettings {
    pub models: ModelPolicy,
    pub routes: Vec<&'static str>,
    pub stages: Vec<&'static str>,
    pub tiers: Vec<&'static str>,
}

impl ModelsSettings {
    pub fn new(models: ModelPolicy) -> ModelsSettings {
        ModelsSettings {
            models,
            routes: BudgetRoute::ALL
                .iter()
                .map(|route| route.as_str())
                .collect(),
            stages: MODEL_STAGES.to_vec(),
            tiers: MODEL_TIERS.to_vec(),
        }
    }
}
