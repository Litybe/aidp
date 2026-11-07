use aidp_agents::McpConfiguration;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// Runtime-only definitions; published snapshots survive replacement of their public identifier.
#[derive(Default)]
pub struct McpConfigurationRegistry {
    pub(crate) configurations: Mutex<BTreeMap<String, Arc<McpConfiguration>>>,
}
