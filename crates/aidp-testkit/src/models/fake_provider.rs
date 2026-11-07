use std::sync::{Arc, Mutex};

use aidp_agents::AgentSpec;

use crate::models::fake_script::FakeScript;

/// Scripted agent provider that runs no LLM and records launched specs.
pub struct FakeProvider {
    pub(crate) script: FakeScript,
    pub(crate) launched: Arc<Mutex<Vec<AgentSpec>>>,
}

impl FakeProvider {
    /// Builds a provider that plays `script` on every start.
    pub fn new(script: FakeScript) -> Self {
        Self {
            script,
            launched: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Specs received by `start`, in order.
    pub fn launched(&self) -> Vec<AgentSpec> {
        self.launched.lock().expect("launched lock").clone()
    }
}
