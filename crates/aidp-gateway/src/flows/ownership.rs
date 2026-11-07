//! Answers the CI autopilot whether a ticket's pull request belongs to a flow.

use super::store::is_active;
use super::FlowEngine;
use aidp_integrations::models::flow_ownership::FlowOwnership;
use std::sync::Arc;

/// Owns a ticket while its flow run is not finished.
pub struct ActiveFlowOwnership {
    engine: Arc<FlowEngine>,
}

impl ActiveFlowOwnership {
    pub fn new(engine: Arc<FlowEngine>) -> Self {
        ActiveFlowOwnership { engine }
    }
}

impl FlowOwnership for ActiveFlowOwnership {
    fn owns(&self, ticket_key: &str) -> bool {
        let direct = self
            .engine
            .executor()
            .store
            .index_get(ticket_key)
            .is_some_and(|entry| is_active(entry.status));
        direct
            || self.engine.list(None, None).into_iter().any(|entry| {
                is_active(entry.status)
                    && self
                        .engine
                        .get(&entry.key)
                        .is_ok_and(|state| super::pipeline_inputs::ticket_key(&state) == ticket_key)
            })
    }
}

#[cfg(test)]
#[path = "../../tests/unit/flow_ownership_tests.rs"]
mod tests;
