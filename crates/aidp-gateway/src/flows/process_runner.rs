use crate::models::constants::aidp::AIDP_PROGRAM;
pub use crate::models::flow_process_runner::FlowProcessRunner;
use aidp_core::process_runner::{ProcessRunner, ProcessSpec, SpawnedChild};
use std::collections::BTreeMap;
use std::sync::Arc;

/// Uses an explicit driver binary, or the running aidp executable when this is the CLI host.
pub fn aidp_program() -> Option<String> {
    std::env::var(crate::models::constants::flow_execution::AIDP_BIN_ENV)
        .ok()
        .filter(|program| !program.trim().is_empty())
        .or_else(|| {
            let executable = std::env::current_exe().ok()?;
            (executable.file_stem()?.to_str()? == AIDP_PROGRAM)
                .then(|| executable.to_string_lossy().to_string())
        })
}

impl FlowProcessRunner {
    pub fn new(
        inner: Arc<dyn ProcessRunner>,
        environment: BTreeMap<String, String>,
        aidp_program: Option<String>,
    ) -> Self {
        Self {
            inner,
            environment,
            aidp_program,
        }
    }

    fn contextual_spec(&self, spec: &ProcessSpec) -> ProcessSpec {
        let mut contextual = spec.clone();
        contextual.envs.extend(self.environment.clone());
        if contextual.program == AIDP_PROGRAM {
            if let Some(program) = &self.aidp_program {
                contextual.program.clone_from(program);
            }
        }
        contextual
    }
}

impl ProcessRunner for FlowProcessRunner {
    fn output(&self, spec: &ProcessSpec) -> std::io::Result<std::process::Output> {
        self.inner.output(&self.contextual_spec(spec))
    }

    fn spawn(&self, spec: &ProcessSpec) -> std::io::Result<Box<dyn SpawnedChild>> {
        self.inner.spawn(&self.contextual_spec(spec))
    }
}

#[cfg(test)]
#[path = "../../tests/unit/flow_process_runner_tests.rs"]
mod tests;
