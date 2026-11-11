//! `aidp memory ingest-knowledge` and `aidp memory inbox`: knowledge proposals and their review.

use super::memory::{extra_roots, global_knowledge_root, open, wants_json, Opened};
use super::CommandOutput;
use crate::args::memory::{MemoryInboxOp, ProjectArg};
use crate::ctx::Ctx;
use aidp_core::ExitStatus;
use aidp_memory::models::constants::knowledge_proposals::{
    KNOWLEDGE_PROPOSALS_FILE, MANUAL_SOURCE,
};
use aidp_memory::{
    load_default_embedder, IndexOptions, KnowledgeInbox, KnowledgeIngestor, MemoryError, Source,
};
use std::path::{Path, PathBuf};

const NO_VAULT: &str =
    "no vault resolved; pass --vault <path> or set AIDP_VAULT (knowledge notes live in the vault)";
const NO_NOTE: &str = "Aucune note en attente.";

fn vault_root(ctx: &Ctx) -> anyhow::Result<PathBuf> {
    ctx.vault_override()
        .map(|vault| vault.root)
        .ok_or_else(|| MemoryError::NotFound(NO_VAULT.to_string()).into())
}

fn repo_of(opened: &Opened) -> String {
    if opened.known_root {
        aidp_core::vault::repo_segment(&opened.project.root)
    } else {
        opened.project.key.clone()
    }
}

fn reindex_knowledge(ctx: &Ctx, opened: &Opened) {
    let embedder = load_default_embedder(&opened.gateway);
    let mut indexer = aidp_memory::Indexer::new(&opened.store);
    if let Some(model) = embedder.as_deref() {
        indexer = indexer.with_embedder(model);
    }
    let options = IndexOptions {
        sources: vec![Source::Knowledge],
        full: false,
        extra_roots: extra_roots(ctx, &opened.project),
        global_knowledge: global_knowledge_root(ctx),
    };
    if !opened.project.root.is_dir() {
        return;
    }
    if let Err(error) = indexer.index(&opened.project, &options) {
        tracing::warn!(%error, "knowledge notes not re-indexed");
    }
}

fn proposals_file(path: &Path) -> PathBuf {
    if path.is_dir() {
        path.join(KNOWLEDGE_PROPOSALS_FILE)
    } else {
        path.to_path_buf()
    }
}

/// Ingests one `knowledge-proposals.json` by hand and re-indexes the knowledge notes.
pub fn ingest_knowledge(
    ctx: &Ctx,
    path: &Path,
    project: &ProjectArg,
) -> anyhow::Result<CommandOutput> {
    let vault = vault_root(ctx)?;
    let opened = open(ctx, project, false)?;
    let file = proposals_file(&ctx.absolute(path));
    let embedder = load_default_embedder(&opened.gateway);
    let mut ingestor = KnowledgeIngestor::new(
        &opened.store,
        &vault,
        &repo_of(&opened),
        &opened.project.key,
    );
    if let Some(model) = embedder.as_deref() {
        ingestor = ingestor.with_embedder(model);
    }
    let report = ingestor.ingest_file(&file, MANUAL_SOURCE)?;
    if report.changed() > 0 {
        reindex_knowledge(ctx, &opened);
    }
    let refused = !report.rejected.is_empty();
    let output = if wants_json(ctx) {
        CommandOutput::json(ctx.format(), serde_json::to_value(&report)?)
    } else {
        let mut lines = vec![format!(
            "{} fichier(s): {} ajoutee(s), {} fusionnee(s), {} en attente, {} reecrite(s), {} remplacee(s), {} refusee(s)",
            report.files,
            report.added,
            report.merged,
            report.inboxed,
            report.updated,
            report.superseded,
            report.rejected.len()
        )];
        lines.extend(
            report
                .results
                .iter()
                .map(|result| format!("- {} {}: {}", result.outcome, result.slug, result.path)),
        );
        lines.extend(
            report
                .rejected
                .iter()
                .map(|entry| format!("- refuse {}: {}", entry.file, entry.reason)),
        );
        CommandOutput::text(lines.join("\n"))
    };
    Ok(if refused {
        output.with_exit(ExitStatus::PolicyRefusal)
    } else {
        output
    })
}

/// Runs one `aidp memory inbox` verb.
pub fn inbox(ctx: &Ctx, op: MemoryInboxOp) -> anyhow::Result<CommandOutput> {
    let vault = vault_root(ctx)?;
    match op {
        MemoryInboxOp::List { project } => {
            let opened = open(ctx, &project, false)?;
            let entries = KnowledgeInbox::new(&vault, &repo_of(&opened)).list()?;
            if wants_json(ctx) {
                return Ok(CommandOutput::json(
                    ctx.format(),
                    serde_json::to_value(&entries)?,
                ));
            }
            if entries.is_empty() {
                return Ok(CommandOutput::text(NO_NOTE));
            }
            let lines: Vec<String> = entries
                .iter()
                .map(|entry| {
                    let link = entry
                        .candidate_of
                        .as_deref()
                        .or(entry.target.as_deref())
                        .map(|path| format!(" -> {path}"))
                        .unwrap_or_default();
                    format!(
                        "{} [{} {}] {}{link}",
                        entry.path, entry.domain, entry.action, entry.title
                    )
                })
                .collect();
            Ok(CommandOutput::text(lines.join("\n")))
        }
        MemoryInboxOp::Approve { path, project } => {
            let opened = open(ctx, &project, false)?;
            let decision = KnowledgeInbox::new(&vault, &repo_of(&opened)).approve(&path)?;
            reindex_knowledge(ctx, &opened);
            decision_output(ctx, &decision)
        }
        MemoryInboxOp::Reject { path, project } => {
            let opened = open(ctx, &project, false)?;
            let decision = KnowledgeInbox::new(&vault, &repo_of(&opened)).reject(&path)?;
            decision_output(ctx, &decision)
        }
    }
}

fn decision_output(
    ctx: &Ctx,
    decision: &aidp_memory::models::inbox_decision::InboxDecision,
) -> anyhow::Result<CommandOutput> {
    if wants_json(ctx) {
        return Ok(CommandOutput::json(
            ctx.format(),
            serde_json::to_value(decision)?,
        ));
    }
    let verb = decision
        .outcome
        .map_or_else(|| "rejected".to_string(), |outcome| outcome.to_string());
    Ok(CommandOutput::text(format!("{verb}: {}", decision.path)))
}
