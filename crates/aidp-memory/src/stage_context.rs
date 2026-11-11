//! The knowledge block of a pipeline stage, built when the pipeline is compiled.

use crate::agent_roles::{domains_for, role_of};
use crate::context::{issue_query_text, ticket_route};
use crate::embedder::Embedder;
use crate::error::Result;
use crate::glossary;
use crate::hybrid::HybridRetriever;
use crate::manager::MemoryManager;
use crate::model_resolver::load_default_embedder;
use crate::models::constants::knowledge::{
    DEMOTED_HEADING_PREFIX, HEADING_PREFIX, KNOWLEDGE_DIR, STAGE_QUERY_SEPARATOR,
};
use crate::models::constants::paths::{TICKETS_DIR, TICKET_EXTENSION};
use crate::models::context_request::ContextRequest;
use crate::models::enums::Route;
use crate::project::resolve_project;
use crate::retriever::{Bm25Retriever, Retriever};
use crate::store::MemoryStore;
use aidp_core::context_provider::ContextProvider;
use aidp_core::models::stage_context_request::StageContextRequest;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

type EmbedderLoader = Box<dyn Fn(&Path) -> Option<Box<dyn Embedder>>>;

struct SharedEmbedder(Arc<dyn Embedder>);

impl Embedder for SharedEmbedder {
    fn model_id(&self) -> &str {
        self.0.model_id()
    }

    fn dim(&self) -> usize {
        self.0.dim()
    }

    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        self.0.embed(texts)
    }
}

/// Builds the knowledge block of a stage from the memory index of the gateway directory.
///
/// Uses the hybrid retriever when an embedder loads, BM25 otherwise. Never fails: an error is
/// logged and gives no block.
pub struct KnowledgeContextProvider {
    gateway_dir: PathBuf,
    vault_root: Option<PathBuf>,
    load_embedder: EmbedderLoader,
    embedder: OnceLock<Option<Arc<dyn Embedder>>>,
}

impl KnowledgeContextProvider {
    /// A provider reading `<gateway_dir>/memory/memory.db`, with the default embedding model.
    pub fn new(gateway_dir: impl Into<PathBuf>) -> Self {
        Self {
            gateway_dir: gateway_dir.into(),
            vault_root: None,
            load_embedder: Box::new(load_default_embedder),
            embedder: OnceLock::new(),
        }
    }

    /// The vault holding the glossary and the mirrored tickets.
    pub fn with_vault(mut self, vault_root: Option<PathBuf>) -> Self {
        self.vault_root = vault_root;
        self
    }

    /// Searches with BM25 only.
    pub fn lexical(mut self) -> Self {
        self.load_embedder = Box::new(|_| None);
        self
    }

    /// Replaces the loader of the embedding model.
    pub fn with_embedder_loader(
        mut self,
        loader: impl Fn(&Path) -> Option<Box<dyn Embedder>> + 'static,
    ) -> Self {
        self.load_embedder = Box::new(loader);
        self
    }

    fn shared_embedder(&self) -> Option<Box<dyn Embedder>> {
        self.embedder
            .get_or_init(|| (self.load_embedder)(&self.gateway_dir).map(Arc::from))
            .clone()
            .map(|embedder| Box::new(SharedEmbedder(embedder)) as Box<dyn Embedder>)
    }

    fn ticket_mirror(&self, project_root: &Path, issue: &str) -> Option<String> {
        let vault = self.vault_root.as_ref()?;
        let repo = aidp_core::vault::repo_segment(project_root);
        let file = vault
            .join(TICKETS_DIR)
            .join(repo)
            .join(format!("{issue}.{TICKET_EXTENSION}"));
        std::fs::read_to_string(file).ok()
    }

    fn build(&self, request: &StageContextRequest) -> Result<Option<String>> {
        let project = resolve_project(None, Some(&self.gateway_dir), &request.project_root)?;
        if let Some(vault) = &self.vault_root {
            glossary::install_from_dir(&vault.join(KNOWLEDGE_DIR));
        }
        let store = MemoryStore::open_in(&self.gateway_dir)?;
        let embedder = self.shared_embedder();
        let retriever: Box<dyn Retriever + '_> = match embedder {
            Some(embedder) => Box::new(HybridRetriever::new(
                &store,
                Some(project.root.clone()),
                embedder,
            )),
            None => Box::new(Bm25Retriever::new(&store, Some(project.root.clone()))),
        };
        let mirror = request
            .issue
            .as_deref()
            .and_then(|issue| self.ticket_mirror(&request.project_root, issue));
        let mut context = ContextRequest::new(project.key);
        context.issue = request.issue.clone();
        context.issue_text = request
            .issue_text
            .clone()
            .or_else(|| mirror.as_deref().map(issue_query_text));
        context.route = request
            .route
            .and_then(|route| Route::parse(route.as_str()).ok())
            .or_else(|| mirror.as_deref().and_then(ticket_route))
            .or_else(|| role_of(&request.agent_type).and_then(|role| role.route));
        context.stage = Some(
            [
                request.stage_id.as_str(),
                request.stage_description.as_str(),
                request.agent_type.as_str(),
            ]
            .into_iter()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(STAGE_QUERY_SEPARATOR),
        );
        context.budget_tokens = request.budget_tokens;
        context.domains = domains_for(&request.agent_type);
        context.agent_type = Some(request.agent_type.clone());
        let block = MemoryManager::new(&store).context(&context, retriever.as_ref())?;
        let markdown = demote_headings(block.markdown.trim());
        Ok((!markdown.is_empty()).then_some(markdown))
    }
}

impl ContextProvider for KnowledgeContextProvider {
    fn stage_context(&self, request: &StageContextRequest) -> Option<String> {
        match self.build(request) {
            Ok(block) => block,
            Err(error) => {
                tracing::warn!(stage = %request.stage_id, %error, "knowledge block not built");
                None
            }
        }
    }
}

fn demote_headings(markdown: &str) -> String {
    markdown
        .lines()
        .map(|line| match line.strip_prefix(HEADING_PREFIX) {
            Some(rest) => format!("{DEMOTED_HEADING_PREFIX}{rest}"),
            None => line.to_string(),
        })
        .collect::<Vec<_>>()
        .join("\n")
}
