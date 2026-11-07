use aidp_gateway::models::constants::engine::{ENGINE_NATIVE, ENGINE_WORKFLOW};
use aidp_gateway::models::enums::engine::Engine;

/// Engine accepted by `aidp gateway start --engine`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum EngineArg {
    /// The daemon drives the run itself.
    #[value(name = ENGINE_NATIVE)]
    Native,
    /// A headless workflow session.
    #[value(name = ENGINE_WORKFLOW)]
    Workflow,
}

impl EngineArg {
    /// The gateway engine this argument selects.
    pub fn into_engine(self) -> Engine {
        match self {
            EngineArg::Native => Engine::Native,
            EngineArg::Workflow => Engine::Workflow,
        }
    }
}

#[cfg(test)]
#[path = "../../../tests/unit/engine_arg_tests.rs"]
mod tests;
