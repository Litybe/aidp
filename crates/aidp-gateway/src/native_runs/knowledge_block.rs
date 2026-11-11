//! The compile-time knowledge block of a native run, frozen when the run starts.

use aidp_core::pipeline;
use aidp_core::run::{self, RunState};
use aidp_memory::{Embedder, KnowledgeContextProvider, SharedEmbedder};
use std::path::Path;
use std::sync::Arc;

/// Fills `state.stage_knowledge` when the pipeline enables `settings.knowledge`.
///
/// `embedder` is the daemon's own running model, never a call back to the daemon: without it the
/// search is lexical. A failure is logged and leaves the prompts without the block.
pub fn freeze(
    gateway_dir: &Path,
    vault_root: Option<&Path>,
    embedder: Option<Arc<dyn Embedder>>,
    state: &mut RunState,
) {
    let pipe = match pipeline::load(&state.pipeline_path) {
        Ok(pipe) => pipe,
        Err(error) => {
            tracing::warn!(%error, "knowledge block skipped: pipeline not loaded");
            return;
        }
    };
    let provider = KnowledgeContextProvider::new(gateway_dir)
        .with_vault(vault_root.map(Path::to_path_buf))
        .with_embedder_loader(move |_| {
            embedder
                .clone()
                .map(|shared| Box::new(SharedEmbedder(shared)) as Box<dyn Embedder>)
        });
    if let Err(error) = run::ensure_stage_knowledge(state, &pipe, Some(&provider)) {
        tracing::warn!(%error, "knowledge block not frozen");
    }
}

#[cfg(test)]
#[path = "../../tests/unit/knowledge_block_tests.rs"]
mod tests;
