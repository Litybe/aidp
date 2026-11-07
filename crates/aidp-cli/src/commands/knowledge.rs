use super::CommandOutput;
use crate::args::{KnowledgeLoc, KnowledgeOp};
use crate::ctx::Ctx;
use aidp_core::knowledge;
use aidp_core::ExitStatus;
use std::path::PathBuf;

const RUN_ENV: &str = "AIDP_RUN";
const LEGACY_KNOWLEDGE_DIR: &str = ".aidp/knowledge";
const DATE_FORMAT: &str = "%Y-%m-%d";
const NO_VAULT_RESOLVED: &str = "no vault resolved; pass --vault <path> or set AIDP_VAULT";

pub fn run(ctx: &Ctx, op: KnowledgeOp) -> anyhow::Result<CommandOutput> {
    let format = ctx.format();
    match op {
        KnowledgeOp::List { loc } => {
            let dir = knowledge_dir_for(ctx, &loc)?;
            let features = knowledge::list(&dir);
            Ok(CommandOutput::json(
                format,
                serde_json::json!({
                    "knowledge_dir": dir,
                    "features_dir": knowledge::features_dir(&dir),
                    "exists": dir.is_dir(),
                    "notes": knowledge::root_notes(&dir),
                    "features": features,
                }),
            ))
        }
        KnowledgeOp::Resolve { names, loc } => {
            let dir = knowledge_dir_for(ctx, &loc)?;
            Ok(CommandOutput::json(
                format,
                serde_json::to_value(knowledge::resolve(&dir, &names))?,
            ))
        }
        KnowledgeOp::Path { loc } => {
            let dir = knowledge_dir_for(ctx, &loc)?;
            Ok(CommandOutput::json(
                format,
                serde_json::json!({
                    "knowledge_dir": dir,
                    "features_dir": knowledge::features_dir(&dir),
                    "exists": dir.is_dir(),
                }),
            ))
        }
        KnowledgeOp::Stats { loc } => {
            let dir = knowledge_dir_for(ctx, &loc)?;
            Ok(CommandOutput::json(
                format,
                serde_json::to_value(knowledge::stats(&dir))?,
            ))
        }
        KnowledgeOp::Supersede {
            slug,
            by,
            reason,
            loc,
        } => supersede(ctx, &slug, &by, reason, &loc),
        KnowledgeOp::Stale { loc } => stale(ctx, &loc),
        KnowledgeOp::Gc { loc } => {
            let dir = knowledge_dir_for(ctx, &loc)?;
            let report = aidp_core::knowledge_gc::report(&dir, &ctx.workspace());
            Ok(CommandOutput::json(format, serde_json::to_value(&report)?))
        }
        KnowledgeOp::Index => index(ctx),
    }
}

/// Resolve the knowledge base directory for a `KnowledgeLoc`.
fn knowledge_dir_for(ctx: &Ctx, loc: &KnowledgeLoc) -> anyhow::Result<PathBuf> {
    if let Some(d) = &loc.knowledge_dir {
        return Ok(d.clone());
    }
    let workspace = ctx.workspace();
    let vault = ctx.vault_override();
    let run = loc.run.clone().or_else(|| std::env::var(RUN_ENV).ok());
    if let Some(run) = run.as_deref().filter(|r| !r.trim().is_empty()) {
        if let Ok(state) = aidp_core::run::load_in(vault.as_ref(), &workspace, run) {
            return Ok(state.knowledge_dir());
        }
    }
    match vault {
        Some(vault) => Ok(vault.knowledge_dir(&aidp_core::vault::repo_segment(&workspace))),
        None => Ok(workspace.join(LEGACY_KNOWLEDGE_DIR)),
    }
}

fn supersede(
    ctx: &Ctx,
    slug: &str,
    by: &str,
    reason: Option<String>,
    loc: &KnowledgeLoc,
) -> anyhow::Result<CommandOutput> {
    knowledge::validate_slug(slug).map_err(|e| aidp_core::TypedExit {
        status: ExitStatus::PolicyRefusal,
        message: format!("invalid --slug: {e}"),
    })?;
    knowledge::validate_slug(by).map_err(|e| aidp_core::TypedExit {
        status: ExitStatus::PolicyRefusal,
        message: format!("invalid --by: {e}"),
    })?;
    let dir = knowledge_dir_for(ctx, loc)?;
    let path = knowledge::features_dir(&dir).join(format!("{slug}.md"));
    let original = std::fs::read_to_string(&path)
        .map_err(|e| anyhow::anyhow!("cannot read {}: {e}", path.display()))?;
    let today = ctx.clock.now_utc().format(DATE_FORMAT).to_string();
    let rewritten = knowledge::supersede_frontmatter(&original, by, reason.as_deref(), &today);
    std::fs::write(&path, rewritten)?;
    Ok(CommandOutput::json(
        ctx.format(),
        serde_json::json!({
            "superseded": path,
            "by": knowledge::features_dir(&dir).join(format!("{by}.md")),
        }),
    ))
}

/// Dates the notes' `files:`, caches the report and returns it.
/// The cache directory is created first so a fresh vault does not fail.
fn stale(ctx: &Ctx, loc: &KnowledgeLoc) -> anyhow::Result<CommandOutput> {
    let dir = knowledge_dir_for(ctx, loc)?;
    let report = aidp_core::knowledge_stale::stale(&dir, &ctx.workspace());
    write_stale_cache(&dir, &report)?;
    Ok(CommandOutput::json(
        ctx.format(),
        serde_json::to_value(&report)?,
    ))
}

/// Writes the report to the cache file, creating the knowledge directory when missing.
fn write_stale_cache(dir: &std::path::Path, report: &impl serde::Serialize) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(
        aidp_core::knowledge_stale::cache_path(dir),
        serde_json::to_string_pretty(report)?,
    )?;
    Ok(())
}

fn index(ctx: &Ctx) -> anyhow::Result<CommandOutput> {
    let resolved = ctx
        .vault_override()
        .ok_or_else(|| anyhow::anyhow!(NO_VAULT_RESOLVED))?;
    let n = aidp_core::obsidian::refresh_knowledge(&resolved.root);
    Ok(CommandOutput::text(format!(
        "reindexed {n} project knowledge base(s) in {}",
        resolved.knowledge_root().display()
    )))
}

#[cfg(test)]
#[path = "../../tests/unit/knowledge_tests.rs"]
mod tests;
