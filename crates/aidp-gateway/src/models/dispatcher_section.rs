use aidp_tickets::config::{DispatcherConfig, NightConfig};
use serde::Serialize;

/// The `dispatcher` keys the budget page edits.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DispatcherSection {
    pub slots: u32,
    pub night: NightConfig,
    pub stop_at_7d_percent: Option<f64>,
}

impl From<&DispatcherConfig> for DispatcherSection {
    fn from(config: &DispatcherConfig) -> Self {
        DispatcherSection {
            slots: config.slots,
            night: config.night.clone(),
            stop_at_7d_percent: config.stop_at_7d_percent,
        }
    }
}
