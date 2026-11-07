use super::CommandOutput;
use crate::args::memory::{MemoryOp, ProjectArg, RagOp};
use crate::ctx::Ctx;
use crate::models::hit_view::HitView;
use aidp_core::{ExitStatus, Format};
use aidp_memory::models::memory_filter::MemoryFilter;
use aidp_memory::models::memory_patch::MemoryPatch;
use aidp_memory::models::memory_ref::MemoryRef;
use aidp_memory::{
    pack, render_markdown, resolve_project, Bm25Retriever, ContextRequest, IndexOptions, IndexRoot,
    Indexer, Memory, MemoryError, MemoryKind, MemoryManager, MemoryStatus, MemoryStore, NewMemory,
    ProjectRef, RefType, Retriever, Route, SearchQuery, Source,
};
use std::path::PathBuf;

const PIPELINES_PREFIX: &str = "Pipelines";
const TICKETS_DIR: &str = "TODO";
const KNOWLEDGE_PREFIX: &str = "Knowledge";
const TICKET_EXTENSION: &str = "md";
const REF_SEPARATOR: char = ':';
const NO_RESULT: &str = "Aucun resultat.";
const NO_MEMORY: &str = "Aucune memoire.";
const ISSUE_NOT_MIRRORED: &str = "issue not found in the ticket mirror; searching without its text";

struct Opened {
    store: MemoryStore,
    project: ProjectRef,
    known_root: bool,
}

/// Runs one `aidp rag` verb.
pub fn run_rag(ctx: &Ctx, op: RagOp) -> anyhow::Result<CommandOutput> {
    match op {
        RagOp::Index {
            project,
            source,
            full,
        } => {
            let opened = open(ctx, &project, true)?;
            let options = IndexOptions {
                sources: parse_sources(source.as_deref())?,
                full,
                extra_roots: extra_roots(ctx, &opened.project),
            };
            let report = Indexer::new(&opened.store).index(&opened.project, &options)?;
            Ok(CommandOutput::json(
                Some(Format::JsonCompact),
                serde_json::to_value(report)?,
            ))
        }
        RagOp::Search {
            query,
            project,
            source,
            path,
            k,
            budget,
        } => {
            let opened = open(ctx, &project, false)?;
            let retriever = retriever(&opened);
            let mut request = SearchQuery::new(opened.project.key.clone(), query);
            request.sources = parse_sources(source.as_deref())?;
            request.path_glob = path;
            request.k = k;
            let packed = pack(retriever.search(&request)?, budget);
            if wants_json(ctx) {
                let views: Vec<HitView> = packed.hits.iter().map(HitView::from).collect();
                return Ok(CommandOutput::json(
                    ctx.format(),
                    serde_json::to_value(views)?,
                ));
            }
            if packed.hits.is_empty() {
                return Ok(CommandOutput::text(NO_RESULT));
            }
            Ok(CommandOutput::text(render_markdown(&packed).trim_end()))
        }
    }
}

/// Runs one `aidp memory` verb.
pub fn run_memory(ctx: &Ctx, op: MemoryOp) -> anyhow::Result<CommandOutput> {
    match op {
        MemoryOp::Context {
            project,
            issue,
            route,
            stage,
            budget,
        } => {
            let opened = open(ctx, &project, false)?;
            let mut request = ContextRequest::new(opened.project.key.clone());
            request.route = route.as_deref().map(Route::parse).transpose()?;
            request.stage = stage;
            request.budget_tokens = budget;
            let mut diagnostics = Vec::new();
            if let Some(issue) = issue {
                match issue_text(ctx, &opened.project, &issue) {
                    Some(text) => request.issue_text = Some(text),
                    None => diagnostics.push(format!("{issue}: {ISSUE_NOT_MIRRORED}")),
                }
                request.issue = Some(issue);
            }
            let manager = MemoryManager::new(&opened.store);
            let block = manager.context(&request, &retriever(&opened))?;
            let output = if wants_json(ctx) {
                CommandOutput::json(ctx.format(), serde_json::to_value(&block)?)
            } else {
                CommandOutput::text(block.markdown.trim_end())
            };
            Ok(output.with_diagnostics(diagnostics))
        }
        MemoryOp::Add {
            project,
            kind,
            title,
            text,
            route,
            paths,
            tags,
            refs,
            status,
            confidence,
            supersedes,
        } => {
            let opened = open(ctx, &project, false)?;
            let mut new = NewMemory::proposal(
                opened.project.key.clone(),
                MemoryKind::parse(&kind)?,
                title,
                text,
            );
            new.status = MemoryStatus::parse(&status)?;
            new.confidence = confidence;
            new.scope.route = route.as_deref().map(Route::parse).transpose()?;
            new.scope.paths = paths;
            new.scope.tags = tags;
            new.refs = refs
                .iter()
                .map(|raw| parse_ref(raw))
                .collect::<Result<_, _>>()?;
            new.supersedes = supersedes;
            let outcome = MemoryManager::new(&opened.store).add(new)?;
            memory_output(ctx, &outcome.memory, serde_json::to_value(&outcome)?)
        }
        MemoryOp::List {
            project,
            status,
            kind,
            q,
        } => {
            let opened = open(ctx, &project, false)?;
            let filter = MemoryFilter {
                status: status.as_deref().map(MemoryStatus::parse).transpose()?,
                kind: kind.as_deref().map(MemoryKind::parse).transpose()?,
                query: q,
            };
            let memories = MemoryManager::new(&opened.store).list(&opened.project.key, &filter)?;
            Ok(list_output(ctx, &memories)?)
        }
        MemoryOp::Show { id, project } => {
            let opened = open(ctx, &project, false)?;
            let memory = MemoryManager::new(&opened.store).get(&opened.project.key, &id)?;
            if wants_json(ctx) {
                return Ok(CommandOutput::json(
                    ctx.format(),
                    serde_json::to_value(&memory)?,
                ));
            }
            Ok(CommandOutput::text(describe(&memory)))
        }
        MemoryOp::Edit {
            id,
            project,
            kind,
            title,
            text,
            route,
            paths,
            tags,
            confidence,
        } => {
            let opened = open(ctx, &project, false)?;
            let patch = MemoryPatch {
                kind: kind.as_deref().map(MemoryKind::parse).transpose()?,
                title,
                text,
                route: route.as_deref().map(Route::parse).transpose()?,
                paths: (!paths.is_empty()).then_some(paths),
                tags: (!tags.is_empty()).then_some(tags),
                confidence,
            };
            let memory =
                MemoryManager::new(&opened.store).edit(&opened.project.key, &id, &patch)?;
            single_output(ctx, &memory)
        }
        MemoryOp::Approve { id, project } => {
            let opened = open(ctx, &project, false)?;
            let memory = MemoryManager::new(&opened.store).approve(&opened.project.key, &id)?;
            single_output(ctx, &memory)
        }
        MemoryOp::Archive { id, project } => {
            let opened = open(ctx, &project, false)?;
            let memory = MemoryManager::new(&opened.store).archive(&opened.project.key, &id)?;
            single_output(ctx, &memory)
        }
        MemoryOp::Merge { id, into, project } => {
            let opened = open(ctx, &project, false)?;
            let memory =
                MemoryManager::new(&opened.store).merge(&opened.project.key, &id, &into)?;
            single_output(ctx, &memory)
        }
        MemoryOp::Gc { project } => {
            let opened = open(ctx, &project, false)?;
            let stale = MemoryManager::new(&opened.store).gc(&opened.project.key)?;
            Ok(list_output(ctx, &stale)?)
        }
        MemoryOp::Ingest { path, project } => {
            let opened = open(ctx, &project, false)?;
            let target = ctx.absolute(&path);
            let report = MemoryManager::new(&opened.store).ingest(&opened.project.key, &target)?;
            let refused = !report.rejected.is_empty();
            let output = if wants_json(ctx) {
                CommandOutput::json(ctx.format(), serde_json::to_value(&report)?)
            } else {
                let mut lines = vec![format!(
                    "{} fichier(s): {} cree(s), {} deja connu(s), {} active(s), {} refuse(s)",
                    report.files,
                    report.created,
                    report.deduplicated,
                    report.activated,
                    report.rejected.len()
                )];
                lines.extend(
                    report
                        .rejected
                        .iter()
                        .map(|entry| format!("- {}: {}", entry.file, entry.reason)),
                );
                CommandOutput::text(lines.join("\n"))
            };
            Ok(if refused {
                output.with_exit(ExitStatus::PolicyRefusal)
            } else {
                output
            })
        }
    }
}

fn open(ctx: &Ctx, argument: &ProjectArg, strict: bool) -> anyhow::Result<Opened> {
    let gateway = aidp_gateway::paths::state_dir()?;
    let workspace = ctx.workspace();
    let resolved = resolve_project(argument.project.as_deref(), Some(&gateway), &workspace);
    let (project, known_root) = match resolved {
        Ok(project) => (project, true),
        Err(MemoryError::NotFound(_)) if !strict => match &argument.project {
            Some(key) => (
                ProjectRef {
                    key: key.clone(),
                    root: workspace,
                },
                false,
            ),
            None => return Err(resolved.unwrap_err().into()),
        },
        Err(error) => return Err(error.into()),
    };
    Ok(Opened {
        store: MemoryStore::open_in(&gateway)?,
        project,
        known_root,
    })
}

fn retriever(opened: &Opened) -> Bm25Retriever<'_> {
    Bm25Retriever::new(
        &opened.store,
        opened.known_root.then(|| opened.project.root.clone()),
    )
}

fn wants_json(ctx: &Ctx) -> bool {
    matches!(
        ctx.format(),
        Some(Format::JsonCompact | Format::JsonPretty | Format::Ndjson)
    )
}

fn parse_sources(raw: Option<&str>) -> Result<Vec<Source>, MemoryError> {
    raw.map_or_else(|| Ok(Vec::new()), Source::parse_list)
}

fn parse_ref(raw: &str) -> Result<MemoryRef, MemoryError> {
    let (kind, id) = raw.split_once(REF_SEPARATOR).ok_or_else(|| {
        MemoryError::Invalid(format!(
            "invalid --ref `{raw}`; expected type:id such as issue:GH-12"
        ))
    })?;
    Ok(MemoryRef {
        ref_type: RefType::parse(kind)?,
        id: id.trim().to_string(),
    })
}

fn extra_roots(ctx: &Ctx, project: &ProjectRef) -> Vec<IndexRoot> {
    let Some(vault) = ctx.vault_override() else {
        return Vec::new();
    };
    let repo = aidp_core::vault::repo_segment(&project.root);
    vec![
        IndexRoot {
            source: Source::Run,
            dir: vault.repo_dir(&repo),
            prefix: format!("{PIPELINES_PREFIX}/{repo}"),
        },
        IndexRoot {
            source: Source::Ticket,
            dir: vault.root.join(TICKETS_DIR).join(&repo),
            prefix: format!("{TICKETS_DIR}/{repo}"),
        },
        IndexRoot {
            source: Source::Knowledge,
            dir: vault.knowledge_dir(&repo),
            prefix: format!("{KNOWLEDGE_PREFIX}/{repo}"),
        },
    ]
}

fn issue_text(ctx: &Ctx, project: &ProjectRef, issue: &str) -> Option<String> {
    let vault = ctx.vault_override()?;
    let repo = aidp_core::vault::repo_segment(&project.root);
    let file: PathBuf = vault
        .root
        .join(TICKETS_DIR)
        .join(repo)
        .join(format!("{issue}.{TICKET_EXTENSION}"));
    std::fs::read_to_string(file).ok()
}

fn describe(memory: &Memory) -> String {
    let refs = memory
        .refs
        .iter()
        .map(|reference| format!("{}:{}", reference.ref_type, reference.id))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "{} [{}] ({}) {}\n{}\nconfiance {:.2}, preuves {}, utilisations {}\nroute: {} | chemins: {} | tags: {}\nrefs: {}\ncree {} | modifie {} | utilise {}",
        memory.id,
        memory.status,
        memory.kind,
        memory.title,
        memory.text,
        memory.confidence,
        memory.evidence,
        memory.uses,
        memory.scope.route.map_or("-", |route| route.as_str()),
        memory.scope.paths.join(", "),
        memory.scope.tags.join(", "),
        refs,
        memory.created_at,
        memory.updated_at,
        memory.last_used_at.as_deref().unwrap_or("jamais"),
    )
}

fn memory_output(
    ctx: &Ctx,
    memory: &Memory,
    json: serde_json::Value,
) -> anyhow::Result<CommandOutput> {
    if wants_json(ctx) {
        return Ok(CommandOutput::json(ctx.format(), json));
    }
    Ok(CommandOutput::text(format!(
        "{} [{}] {}",
        memory.id, memory.status, memory.title
    )))
}

fn single_output(ctx: &Ctx, memory: &Memory) -> anyhow::Result<CommandOutput> {
    memory_output(ctx, memory, serde_json::to_value(memory)?)
}

fn list_output(ctx: &Ctx, memories: &[Memory]) -> anyhow::Result<CommandOutput> {
    if wants_json(ctx) {
        return Ok(CommandOutput::json(
            ctx.format(),
            serde_json::to_value(memories)?,
        ));
    }
    if memories.is_empty() {
        return Ok(CommandOutput::text(NO_MEMORY));
    }
    let mut lines = vec![
        "| id | statut | type | preuves | titre |".to_string(),
        "|---|---|---|---|---|".to_string(),
    ];
    lines.extend(memories.iter().map(|memory| {
        format!(
            "| {} | {} | {} | {} | {} |",
            memory.id, memory.status, memory.kind, memory.evidence, memory.title
        )
    }));
    Ok(CommandOutput::text(lines.join("\n")))
}
