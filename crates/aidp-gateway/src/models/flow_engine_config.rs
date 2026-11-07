use crate::flows::DEFAULT_FLOW;
use std::collections::BTreeSet;
use std::path::PathBuf;

/// Where flow definitions come from and what the validator may assume.
#[derive(Clone)]
pub struct FlowEngineConfig {
    /// Directories holding `<flow>.yaml`, searched in order.
    pub flow_dirs: Vec<PathBuf>,
    pub default_flow: String,
    /// Pipeline templates a `pipeline` step may name; `None` when the catalog is unreadable.
    pub pipeline_templates: Option<BTreeSet<String>>,
}

impl Default for FlowEngineConfig {
    fn default() -> Self {
        FlowEngineConfig {
            flow_dirs: Vec::new(),
            default_flow: DEFAULT_FLOW.to_string(),
            pipeline_templates: None,
        }
    }
}
