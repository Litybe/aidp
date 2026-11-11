//! The retriever every entry point builds the same way: hybrid with a model, BM25 without.

use crate::embedder::Embedder;
use crate::glossary;
use crate::hybrid::HybridRetriever;
use crate::models::constants::knowledge::KNOWLEDGE_DIR;
use crate::retriever::{Bm25Retriever, Retriever};
use crate::store::MemoryStore;
use std::path::{Path, PathBuf};

/// Hybrid retrieval when an embedder is given, BM25 and graph otherwise.
pub fn build_retriever<'a>(
    store: &'a MemoryStore,
    project_root: Option<PathBuf>,
    embedder: Option<Box<dyn Embedder>>,
) -> Box<dyn Retriever + 'a> {
    match embedder {
        Some(embedder) => Box::new(HybridRetriever::new(store, project_root, embedder)),
        None => Box::new(Bm25Retriever::new(store, project_root)),
    }
}

/// Installs the glossary of `<vault>/Knowledge/_glossary.md` for this process; the last call wins.
pub fn install_vault_glossary(vault_root: &Path) {
    glossary::install_from_dir(&vault_root.join(KNOWLEDGE_DIR));
}
