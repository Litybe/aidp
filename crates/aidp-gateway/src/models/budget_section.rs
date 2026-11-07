use aidp_tickets::config::{BudgetConfig, PerTicketBudget, PixellabBudget};
use serde::Serialize;

/// The `budget` keys the settings page edits.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct BudgetSection {
    pub max_utilization_5h: f64,
    pub max_utilization_7d: f64,
    pub reserve_per_run_5h: f64,
    pub reserve_per_run_7d: f64,
    pub unknown_policy: String,
    pub learn: bool,
    pub per_ticket: PerTicketBudget,
    pub pixellab: PixellabBudget,
}

impl From<&BudgetConfig> for BudgetSection {
    fn from(config: &BudgetConfig) -> Self {
        BudgetSection {
            max_utilization_5h: config.max_utilization_5h,
            max_utilization_7d: config.max_utilization_7d,
            reserve_per_run_5h: config.reserve_per_run_5h,
            reserve_per_run_7d: config.reserve_per_run_7d,
            unknown_policy: config.unknown_policy.clone(),
            learn: config.learn,
            per_ticket: config.per_ticket.clone(),
            pixellab: config.pixellab.clone(),
        }
    }
}
