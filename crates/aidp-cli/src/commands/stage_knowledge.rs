//! Freezes the compile-time knowledge block of each agent stage before a script is emitted.

use crate::ctx::Ctx;
use aidp_core::models::knowledge_settings::KnowledgeSettings;
use aidp_core::run::RunState;
use aidp_memory::KnowledgeContextProvider;

/// Fills `run_state.stage_knowledge` when the pipeline enables `settings.knowledge`.
///
/// Does nothing under `--no-knowledge`. A missing gateway directory only skips the block.
pub fn freeze(ctx: &Ctx, run_state: &mut RunState) -> anyhow::Result<()> {
    if ctx.globals.no_knowledge {
        return Ok(());
    }
    let pipe = aidp_core::pipeline::load(&run_state.pipeline_path)?;
    if !KnowledgeSettings::from_settings(&pipe.settings).enabled {
        return Ok(());
    }
    let gateway = match aidp_gateway::paths::state_dir() {
        Ok(gateway) => gateway,
        Err(error) => {
            tracing::warn!(%error, "knowledge block skipped: no gateway directory");
            return Ok(());
        }
    };
    let provider = KnowledgeContextProvider::new(gateway)
        .with_vault(ctx.vault_override().map(|vault| vault.root));
    aidp_core::run::ensure_stage_knowledge(run_state, &pipe, Some(&provider))
}
