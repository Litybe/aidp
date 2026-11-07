use super::status::{AIDP_DIR, PIPELINES_DIR};
use super::CommandOutput;
use crate::args::VaultOp;
use crate::ctx::Ctx;
use aidp_core::run::RUN_STATE_FILE;
use std::path::{Path, PathBuf};

const DEFAULT_VAULT_REMEDY: &str = "no vault resolved. Pass --vault <path>, set AIDP_VAULT, or create an Obsidian vault next to your repos (a directory containing .obsidian/).";
const REINDEX_NO_VAULT: &str = "no vault resolved; pass --vault <path> or set AIDP_VAULT";
const OBSIDIAN_DIR: &str = ".obsidian";
const TODO_DIR: &str = "TODO";

pub fn run(ctx: &Ctx, op: VaultOp) -> anyhow::Result<CommandOutput> {
    match op {
        VaultOp::Status => status(
            ctx,
            ctx.format(),
            ctx.workspace_root_flag().map(Path::to_path_buf),
            ctx.vault_flag().map(Path::to_path_buf),
        ),
        VaultOp::Init { no_config } => init(
            ctx,
            ctx.vault_flag().map(Path::to_path_buf),
            ctx.workspace_root_flag().map(Path::to_path_buf),
            no_config,
        ),
        VaultOp::Reindex => reindex(
            ctx,
            ctx.vault_flag().map(Path::to_path_buf),
            ctx.workspace_root_flag().map(Path::to_path_buf),
        ),
    }
}

/// Everything `aidp vault status` reports — also reused by `doctor`.
#[derive(serde::Serialize)]
pub struct VaultStatus {
    pub configured: bool,
    pub root: Option<String>,
    pub source: Option<&'static str>,
    pub exists: bool,
    pub is_obsidian_vault: bool,
    pub pipelines_dir: Option<String>,
    /// `<vault>/Knowledge/<repo>`, the durable per-project knowledge base.
    /// Reported next to the run layout so users see whether the project is documented.
    pub knowledge_dir: Option<String>,
    pub knowledge_topics: usize,
    pub repo_segment: String,
    pub repos: usize,
    pub runs: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ambiguity: Option<String>,
    pub config_file: Option<String>,
    /// Legacy in-repo runs still readable from this workspace.
    pub legacy_runs: usize,
}

pub fn collect_vault_status(workspace_root: &Path, explicit: Option<&Path>) -> VaultStatus {
    let resolved_vault = aidp_core::vault::resolve(explicit, workspace_root);
    let (mut repos, mut runs) = (0usize, 0usize);
    if let Some(vault) = &resolved_vault {
        let pd = vault.pipelines_dir();
        for repo in read_child_dirs(&pd) {
            repos += 1;
            for feature in read_child_dirs(&repo) {
                runs += read_child_dirs(&feature)
                    .iter()
                    .filter(|r| r.join(RUN_STATE_FILE).is_file())
                    .count();
            }
        }
    }
    let repo_seg = aidp_core::vault::repo_segment(workspace_root);
    let knowledge = resolved_vault
        .as_ref()
        .map(|vault| vault.knowledge_dir(&repo_seg));
    let knowledge_topics = knowledge
        .as_deref()
        .map(|d| read_child_dirs(d).len())
        .unwrap_or(0);
    let legacy = workspace_root.join(AIDP_DIR).join(PIPELINES_DIR);
    let legacy_runs = read_child_dirs(&legacy)
        .iter()
        .map(|d| {
            if d.join(RUN_STATE_FILE).is_file() {
                1
            } else {
                read_child_dirs(d)
                    .iter()
                    .filter(|r| r.join(RUN_STATE_FILE).is_file())
                    .count()
            }
        })
        .sum();

    VaultStatus {
        configured: resolved_vault.is_some(),
        root: resolved_vault
            .as_ref()
            .map(|vault| vault.root.display().to_string()),
        source: resolved_vault.as_ref().map(|vault| vault.source.as_str()),
        exists: resolved_vault
            .as_ref()
            .map(|vault| vault.root.is_dir())
            .unwrap_or(false),
        is_obsidian_vault: resolved_vault
            .as_ref()
            .map(|vault| vault.root.join(".obsidian").is_dir())
            .unwrap_or(false),
        pipelines_dir: resolved_vault
            .as_ref()
            .map(|vault| vault.pipelines_dir().display().to_string()),
        knowledge_dir: knowledge.as_ref().map(|d| d.display().to_string()),
        knowledge_topics,
        repo_segment: repo_seg,
        repos,
        runs,
        ambiguity: resolved_vault
            .as_ref()
            .and_then(|vault| vault.ambiguity.clone()),
        config_file: aidp_core::vault::config_file().map(|p| p.display().to_string()),
        legacy_runs,
    }
}

fn read_child_dirs(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return vec![];
    };
    let mut paths: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .filter(|p| {
            p.file_name()
                .and_then(|s| s.to_str())
                .map(|n| !n.starts_with('.'))
                .unwrap_or(false)
        })
        .collect();
    paths.sort();
    paths
}

fn status(
    ctx: &Ctx,
    format: Option<aidp_core::Format>,
    workspace_root: Option<PathBuf>,
    vault: Option<PathBuf>,
) -> anyhow::Result<CommandOutput> {
    let workspace = ctx.cwd_or(workspace_root);
    let vault_status = collect_vault_status(&workspace, vault.as_deref());
    let value = serde_json::to_value(&vault_status)?;
    let rendered = crate::output::render_value(format.unwrap_or_default(), &value)?;
    Ok(CommandOutput::text(rendered))
}

/// Merges the vault path into `~/.aidp/config.json`, keeping every other key.
/// The file is shared with anything else that may live under `~/.aidp/`.
fn write_vault_config(vault_root: &Path) -> anyhow::Result<PathBuf> {
    let cfg = aidp_core::vault::config_file().ok_or_else(|| {
        anyhow::anyhow!("cannot locate the home directory for ~/.aidp/config.json")
    })?;
    if let Some(parent) = cfg.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut doc: serde_json::Value = std::fs::read_to_string(&cfg)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    if let serde_json::Value::Object(m) = &mut doc {
        m.insert(
            "vault".into(),
            serde_json::Value::String(vault_root.display().to_string()),
        );
    }
    std::fs::write(&cfg, serde_json::to_vec_pretty(&doc)?)?;
    Ok(cfg)
}

fn init(
    ctx: &Ctx,
    vault: Option<PathBuf>,
    workspace_root: Option<PathBuf>,
    no_config: bool,
) -> anyhow::Result<CommandOutput> {
    let workspace = ctx.cwd_or(workspace_root);
    let resolved = aidp_core::vault::resolve(vault.as_deref(), &workspace)
        .ok_or_else(|| anyhow::anyhow!(DEFAULT_VAULT_REMEDY))?;
    resolved.ensure_skeleton()?;
    let obsidian = resolved.root.join(OBSIDIAN_DIR);
    let created_vault_marker = !obsidian.exists();
    std::fs::create_dir_all(&obsidian)?;
    aidp_core::obsidian::ensure_dashboard(&resolved.root);

    let mut lines: Vec<String> = Vec::new();
    if !no_config {
        let cfg = write_vault_config(&resolved.root)?;
        lines.push(format!("config:  {}", cfg.display()));
    }
    lines.push(format!(
        "vault:   {} (via {})",
        resolved.root.display(),
        resolved.source.as_str()
    ));
    lines.push(format!("layout:  {}", resolved.pipelines_dir().display()));
    lines.push(format!(
        "knowledge: {}",
        resolved.knowledge_root().display()
    ));
    if created_vault_marker {
        lines.push(
            "note:    created .obsidian/ — open this folder in Obsidian to use it as a vault"
                .to_string(),
        );
    }
    Ok(CommandOutput::text(lines.join("\n")))
}

/// Regenerates the notes of every run in the vault and returns how many were refreshed.
/// A run that fails to refresh is reported in `diagnostics` and skipped.
fn reindex_runs(
    resolved: &aidp_core::models::vault::Vault,
    diagnostics: &mut Vec<String>,
) -> usize {
    let mut refreshed = 0usize;
    for repo in read_child_dirs(&resolved.pipelines_dir()) {
        for feature in read_child_dirs(&repo) {
            for run in read_child_dirs(&feature) {
                let state_file = run.join(RUN_STATE_FILE);
                if !state_file.is_file() {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(&state_file) else {
                    continue;
                };
                let Ok(mut run_state) = serde_json::from_str::<aidp_core::run::RunState>(&text)
                else {
                    continue;
                };
                if run_state.vault_root.is_none() {
                    run_state.vault_root = Some(resolved.root.clone());
                }
                match aidp_core::obsidian::refresh(&run_state) {
                    Ok(()) => refreshed += 1,
                    Err(error) => {
                        diagnostics
                            .push(format!("aidp: vault reindex: {}: {error}", run.display()));
                    }
                }
            }
        }
    }
    refreshed
}

/// Refreshes the ticket board and returns the ticket count, or 0 when there is none.
/// Best-effort: a vault with no `TODO/`, or a broken one, never turns a reindex red.
fn reindex_tickets(root: &Path, diagnostics: &mut Vec<String>) -> usize {
    if !root.join(TODO_DIR).is_dir() {
        return 0;
    }
    match aidp_tickets::store::TicketStore::open(root) {
        Ok(store) => {
            let tickets = store.list(&aidp_tickets::store::ListFilter {
                archived: true,
                ..Default::default()
            });
            let report = aidp_tickets::index::refresh(root, &tickets);
            if report.failed.is_empty() {
                tickets.len()
            } else {
                diagnostics.push(format!(
                    "aidp: vault reindex: ticket board refresh failed: {}",
                    report.warnings().join("; ")
                ));
                0
            }
        }
        Err(e) => {
            diagnostics.push(format!(
                "aidp: vault reindex: failed to open ticket store: {e}"
            ));
            0
        }
    }
}

fn reindex(
    ctx: &Ctx,
    vault: Option<PathBuf>,
    workspace_root: Option<PathBuf>,
) -> anyhow::Result<CommandOutput> {
    let workspace = ctx.cwd_or(workspace_root);
    let resolved = aidp_core::vault::resolve(vault.as_deref(), &workspace)
        .ok_or_else(|| anyhow::anyhow!(REINDEX_NO_VAULT))?;
    let mut diagnostics = Vec::new();
    let runs = reindex_runs(&resolved, &mut diagnostics);
    aidp_core::obsidian::ensure_dashboard(&resolved.root);
    let knowledge_bases = aidp_core::obsidian::refresh_knowledge(&resolved.root);
    let tickets = reindex_tickets(&resolved.root, &mut diagnostics);
    Ok(CommandOutput::text(format!(
        "reindexed {runs} run(s), {knowledge_bases} knowledge base(s) and {tickets} ticket(s) in {}",
        resolved.root.display()
    ))
    .with_diagnostics(diagnostics))
}
