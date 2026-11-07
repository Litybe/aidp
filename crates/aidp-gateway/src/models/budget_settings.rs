use crate::models::budget_section::BudgetSection;
use crate::models::dispatcher_section::DispatcherSection;
use aidp_tickets::config::Config;
use serde::Serialize;

/// Body of `GET /api/settings/budget`: the YAML keys of `TODO/config.yaml`, unchanged.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BudgetSettings {
    pub budget: BudgetSection,
    pub dispatcher: DispatcherSection,
}

impl From<&Config> for BudgetSettings {
    fn from(config: &Config) -> Self {
        BudgetSettings {
            budget: BudgetSection::from(&config.budget),
            dispatcher: DispatcherSection::from(&config.dispatcher),
        }
    }
}
