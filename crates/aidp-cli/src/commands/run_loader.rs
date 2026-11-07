use super::CommandOutput;
use crate::ctx::Ctx;
use aidp_core::run::RunState;
use aidp_core::{Error, ExitStatus};

const VAULT_SEARCHED_LABEL: &str = "vault searched:";
const UNKNOWN_GATE_MESSAGE: &str = "unknown gate";
const RUN_STAGES_LABEL: &str = "stages of this run:";

/// Loads a run using the global vault, so a run written with `--vault` reads back with it.
/// A run that is not found names the vault root that was searched.
pub fn load_run(ctx: &Ctx, workspace: &std::path::Path, run: &str) -> anyhow::Result<RunState> {
    let vault = ctx.vault_override();
    aidp_core::run::load_in(vault.as_ref(), workspace, run).map_err(|error| {
        let Some(Error::NotFound(message)) = error.downcast_ref::<Error>() else {
            return error;
        };
        let searched = match &vault {
            Some(found) => format!("{VAULT_SEARCHED_LABEL} {}", found.root.display()),
            None => format!("{VAULT_SEARCHED_LABEL} none"),
        };
        anyhow::Error::new(Error::NotFound(format!("{message} ({searched})")))
    })
}

/// A usage refusal when `gate` names no stage of the run, listing the stage names.
/// Callers check this before any write.
pub fn unknown_gate_refusal(run_state: &RunState, gate: &str) -> Option<CommandOutput> {
    if run_state.stages.iter().any(|stage| stage.name == gate) {
        return None;
    }
    let names: Vec<&str> = run_state
        .stages
        .iter()
        .map(|stage| stage.name.as_str())
        .collect();
    let message = format!(
        "{UNKNOWN_GATE_MESSAGE} `{gate}`; {RUN_STAGES_LABEL} {}",
        names.join(", ")
    );
    Some(
        CommandOutput::empty()
            .with_diagnostics(vec![format!("error: {message}")])
            .with_exit(ExitStatus::Usage),
    )
}

#[cfg(test)]
#[path = "../../tests/unit/run_loader_tests.rs"]
mod tests;
