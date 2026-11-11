use super::memory_knowledge;
use super::CommandOutput;
use crate::args::memory::{MemoryModelOp, MemoryOp, ProjectArg, RagOp};
use crate::ctx::Ctx;
use crate::models::hit_view::HitView;
use aidp_core::{ExitStatus, Format};
use aidp_memory::agent_roles::{narrow_by_route, parse_domains, role_of};
use aidp_memory::embedding_index::{embed_pending, lacks_vectors};
use aidp_memory::glossary;
use aidp_memory::models::constants::paths::NOTES_DIR;
use aidp_memory::models::constants::schema::GLOBAL_PROJECT;
use aidp_memory::models::memory_filter::MemoryFilter;
use aidp_memory::models::memory_patch::MemoryPatch;
use aidp_memory::models::memory_ref::MemoryRef;
use aidp_memory::{
    build_retriever, is_registered, load_default_embedder, model_status, pack, render_markdown,
    resolve_project, ContextRequest, IndexOptions, IndexRoot, Indexer, Memory, MemoryError,
    MemoryKind, MemoryManager, MemoryStatus, MemoryStore, NewMemory, ProjectRef, RefType,
    Retriever, Route, SearchQuery, Source,
};
use std::path::PathBuf;

const PIPELINES_PREFIX: &str = "Pipelines";
const TICKETS_DIR: &str = "TODO";
const KNOWLEDGE_PREFIX: &str = "Knowledge";
const TICKET_EXTENSION: &str = "md";
const REF_SEPARATOR: char = ':';
const NO_RESULT: &str = "Aucun resultat.";
const NO_MEMORY: &str = "Aucune memoire.";
const MODEL_REQUIRED: &str =
    "--embed needs the embedding model; install it under the memory models folder (see `aidp memory model status`)";
const ISSUE_NOT_MIRRORED: &str = "issue not found in the ticket mirror; searching without its text";

pub(super) struct Opened {
    pub(super) gateway: PathBuf,
    pub(super) store: MemoryStore,
    pub(super) project: ProjectRef,
    pub(super) known_root: bool,
}

/// Runs one `aidp rag` verb.
pub fn run_rag(ctx: &Ctx, op: RagOp) -> anyhow::Result<CommandOutput> {
    match op {
        RagOp::Index {
            project,
            source,
            full,
            embed,
        } => {
            let opened = open(ctx, &project, true)?;
            let options = IndexOptions {
                sources: parse_sources(source.as_deref())?,
                full,
                extra_roots: extra_roots(ctx, &opened.project),
                global_knowledge: global_knowledge_root(ctx),
            };
            let embedder = load_default_embedder(&opened.gateway);
            if embed && embedder.is_none() {
                return Err(MemoryError::Invalid(MODEL_REQUIRED.to_string()).into());
            }
            let mut indexer = Indexer::new(&opened.store);
            if let Some(embedder) = embedder.as_deref() {
                indexer = indexer.with_embedder(embedder);
            }
            let report = indexer.index(&opened.project, &options)?;
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
            lexical,
            domains,
            agent_type,
        } => {
            let opened = open(ctx, &project, false)?;
            let retriever = retriever(&opened, lexical);
            let mut request = SearchQuery::new(opened.project.key.clone(), query);
            request.sources = parse_sources(source.as_deref())?;
            request.path_glob = path;
            request.k = k;
            request.domains = parse_domains(&domains, agent_type.as_deref())?;
            request.agent_type = agent_type;
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
        RagOp::Eval {
            cases,
            project,
            k,
            lexical,
        } => {
            let opened = open(ctx, &project, false)?;
            let content = aidp_core::text::read_text_lenient(ctx.absolute(&cases))?;
            let mut parsed = aidp_memory::eval::parse_cases(&content)?;
            for case in &mut parsed {
                if let Some(text) = case
                    .issue
                    .as_deref()
                    .and_then(|issue| issue_text(ctx, &opened.project, issue))
                {
                    case.query = format!("{text} {}", case.query);
                }
                case.issue_route = case
                    .issue
                    .as_deref()
                    .and_then(|issue| issue_route(ctx, &opened.project, issue));
            }
            let manager = MemoryManager::new(&opened.store);
            let report = aidp_memory::eval::evaluate(
                &manager,
                retriever(&opened, lexical).as_ref(),
                &opened.project.key,
                &parsed,
                k,
            )?;
            Ok(CommandOutput::json(
                ctx.format(),
                serde_json::to_value(report)?,
            ))
        }
        RagOp::Purge { key } => {
            let gateway = aidp_gateway::paths::state_dir()?;
            if is_registered(&key, Some(&gateway)) {
                return Err(MemoryError::Conflict(format!(
                    "`{key}` is a registered project; its index is not purged"
                ))
                .into());
            }
            let report = MemoryStore::open_in(&gateway)?.purge_index(&key)?;
            Ok(CommandOutput::json(
                Some(Format::JsonCompact),
                serde_json::to_value(report)?,
            ))
        }
    }
}

/// Runs one `aidp memory` verb and keeps the vault notes in step with the database.
pub fn run_memory(ctx: &Ctx, op: MemoryOp) -> anyhow::Result<CommandOutput> {
    let read_notes_first = matches!(op, MemoryOp::Context { .. } | MemoryOp::Recall { .. });
    let synced = synced_project(&op);
    if let Some(project) = synced.as_ref().filter(|_| read_notes_first) {
        sync_notes_quietly(ctx, project);
    }
    let output = dispatch_memory(ctx, op)?;
    if let Some(project) = synced.as_ref().filter(|_| !read_notes_first) {
        sync_notes_quietly(ctx, project);
    }
    Ok(output)
}

fn synced_project(op: &MemoryOp) -> Option<ProjectArg> {
    match op {
        MemoryOp::Context { project, .. }
        | MemoryOp::Recall { project, .. }
        | MemoryOp::Add { project, .. }
        | MemoryOp::Edit { project, .. }
        | MemoryOp::Approve { project, .. }
        | MemoryOp::Archive { project, .. }
        | MemoryOp::Merge { project, .. }
        | MemoryOp::Gc { project }
        | MemoryOp::Distill { project, .. }
        | MemoryOp::Ingest { project, .. } => Some(project.clone()),
        MemoryOp::IngestKnowledge { .. }
        | MemoryOp::Inbox { .. }
        | MemoryOp::List { .. }
        | MemoryOp::Show { .. }
        | MemoryOp::Sync { .. }
        | MemoryOp::Model { .. }
        | MemoryOp::Hook { .. } => None,
    }
}

pub(super) fn knowledge_dir(ctx: &Ctx, opened: &Opened) -> Option<PathBuf> {
    let vault = ctx.vault_override()?;
    let repo = if opened.known_root {
        aidp_core::vault::repo_segment(&opened.project.root)
    } else {
        opened.project.key.clone()
    };
    Some(vault.knowledge_dir(&repo))
}

fn notes_dir(ctx: &Ctx, opened: &Opened) -> Option<PathBuf> {
    Some(knowledge_dir(ctx, opened)?.join(NOTES_DIR))
}

fn sync_notes_quietly(ctx: &Ctx, project: &ProjectArg) {
    let outcome = open(ctx, project, false).and_then(|opened| {
        if let Some(dir) = notes_dir(ctx, &opened) {
            let report = MemoryManager::new(&opened.store).sync_vault(&opened.project.key, &dir)?;
            for rejection in report.rejected {
                tracing::warn!(file = %rejection.file, "memory note refused: {}", rejection.reason);
            }
        }
        embed_memories(&opened)
    });
    if let Err(error) = outcome {
        tracing::warn!(%error, "memory notes not synchronised");
    }
}

fn embed_memories(opened: &Opened) -> anyhow::Result<()> {
    let key = opened.project.key.as_str();
    if !lacks_vectors(&opened.store, key, Source::Memory)? {
        return Ok(());
    }
    if let Some(embedder) = load_default_embedder(&opened.gateway) {
        embed_pending(&opened.store, embedder.as_ref(), key, Some(Source::Memory))?;
    }
    Ok(())
}

fn dispatch_memory(ctx: &Ctx, op: MemoryOp) -> anyhow::Result<CommandOutput> {
    match op {
        MemoryOp::Hook { event } => Ok(super::memory_hook::run(ctx, event)),
        MemoryOp::IngestKnowledge { path, project } => {
            memory_knowledge::ingest_knowledge(ctx, &path, &project)
        }
        MemoryOp::Inbox { op } => memory_knowledge::inbox(ctx, op),
        MemoryOp::Recall {
            query,
            project,
            route,
            budget,
            lexical,
        } => {
            let opened = open(ctx, &project, false)?;
            let mut request = ContextRequest::new(opened.project.key.clone());
            request.issue_text = Some(query);
            request.route = route.as_deref().map(Route::parse).transpose()?;
            request.budget_tokens = budget;
            let block = MemoryManager::new(&opened.store)
                .context(&request, retriever(&opened, lexical).as_ref())?;
            if wants_json(ctx) {
                return Ok(CommandOutput::json(
                    ctx.format(),
                    serde_json::to_value(&block)?,
                ));
            }
            if block.markdown.trim().is_empty() {
                return Ok(CommandOutput::text(NO_RESULT));
            }
            Ok(CommandOutput::text(block.markdown.trim_end()))
        }
        MemoryOp::Distill {
            journal,
            session,
            project,
        } => super::memory_hook::distill(ctx, &journal, &session, &project),
        MemoryOp::Model { op } => match op {
            MemoryModelOp::Status => {
                let gateway = aidp_gateway::paths::state_dir()?;
                let store = MemoryStore::open_in(&gateway)?;
                let status = model_status(&store, &gateway)?;
                if wants_json(ctx) {
                    return Ok(CommandOutput::json(
                        ctx.format(),
                        serde_json::to_value(&status)?,
                    ));
                }
                Ok(CommandOutput::text(format!(
                    "modele: {}
chemin: {}
installe: {}
dimension: {}
chunks vectorises: {} / {}
chunks d'un autre modele: {}",
                    status.model,
                    status.path.display(),
                    if status.present { "oui" } else { "non" },
                    status
                        .dim
                        .map_or_else(|| "inconnue".to_string(), |dim| dim.to_string()),
                    status.embedded_chunks,
                    status.total_chunks,
                    status.stale_chunks,
                )))
            }
        },
        MemoryOp::Sync { project } => {
            let opened = open(ctx, &project, false)?;
            let dir = notes_dir(ctx, &opened).ok_or_else(|| {
                MemoryError::NotFound("no vault: the notes have no folder".to_string())
            })?;
            let report = MemoryManager::new(&opened.store).sync_vault(&opened.project.key, &dir)?;
            embed_memories(&opened)?;
            let refused = !report.rejected.is_empty();
            let output =
                CommandOutput::json(Some(Format::JsonCompact), serde_json::to_value(report)?);
            Ok(if refused {
                output.with_exit(ExitStatus::PolicyRefusal)
            } else {
                output
            })
        }
        MemoryOp::Context {
            project,
            issue,
            route,
            stage,
            budget,
            lexical,
            domains,
            agent_type,
        } => {
            let opened = open(ctx, &project, false)?;
            let mut request = ContextRequest::new(opened.project.key.clone());
            request.route = route.as_deref().map(Route::parse).transpose()?;
            request.stage = stage;
            request.budget_tokens = budget;
            request.domains = parse_domains(&domains, agent_type.as_deref())?;
            request.agent_type = agent_type;
            let mut diagnostics = Vec::new();
            if let Some(issue) = issue {
                match issue_text(ctx, &opened.project, &issue) {
                    Some(text) => request.issue_text = Some(text),
                    None => diagnostics.push(format!("{issue}: {ISSUE_NOT_MIRRORED}")),
                }
                if request.route.is_none() {
                    request.route = issue_route(ctx, &opened.project, &issue);
                }
                request.issue = Some(issue);
            }
            if request.route.is_none() {
                request.route = request
                    .agent_type
                    .as_deref()
                    .and_then(role_of)
                    .and_then(|role| role.route);
            }
            request.domains = narrow_by_route(
                std::mem::take(&mut request.domains),
                request.agent_type.as_deref(),
                request.route,
            );
            let manager = MemoryManager::new(&opened.store);
            let block = manager.context(&request, retriever(&opened, lexical).as_ref())?;
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
            let route_of = |issue: &str| issue_route(ctx, &opened.project, issue);
            let report = MemoryManager::new(&opened.store).ingest_routed(
                &opened.project.key,
                &target,
                &route_of,
            )?;
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

pub(super) fn open(ctx: &Ctx, argument: &ProjectArg, strict: bool) -> anyhow::Result<Opened> {
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
    if let Some(vault) = ctx.vault_override() {
        glossary::install_from_dir(&vault.root.join(KNOWLEDGE_PREFIX));
    }
    Ok(Opened {
        store: MemoryStore::open_in(&gateway)?,
        gateway,
        project,
        known_root,
    })
}

pub(super) fn retriever(opened: &Opened, lexical: bool) -> Box<dyn Retriever + '_> {
    let root = opened.known_root.then(|| opened.project.root.clone());
    let embedder = if lexical {
        None
    } else {
        load_default_embedder(&opened.gateway)
    };
    build_retriever(&opened.store, root, embedder)
}

pub(super) fn wants_json(ctx: &Ctx) -> bool {
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

pub(super) fn extra_roots(ctx: &Ctx, project: &ProjectRef) -> Vec<IndexRoot> {
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

/// `<vault>/Knowledge/_global`, indexed under the `_global` project.
pub(super) fn global_knowledge_root(ctx: &Ctx) -> Option<IndexRoot> {
    let vault = ctx.vault_override()?;
    Some(IndexRoot {
        source: Source::Knowledge,
        dir: vault.root.join(KNOWLEDGE_PREFIX).join(GLOBAL_PROJECT),
        prefix: format!("{KNOWLEDGE_PREFIX}/{GLOBAL_PROJECT}"),
    })
}

fn ticket_mirror(ctx: &Ctx, project: &ProjectRef, issue: &str) -> Option<String> {
    let vault = ctx.vault_override()?;
    let repo = aidp_core::vault::repo_segment(&project.root);
    let file: PathBuf = vault
        .root
        .join(TICKETS_DIR)
        .join(repo)
        .join(format!("{issue}.{TICKET_EXTENSION}"));
    std::fs::read_to_string(file).ok()
}

fn issue_text(ctx: &Ctx, project: &ProjectRef, issue: &str) -> Option<String> {
    ticket_mirror(ctx, project, issue).map(|mirror| aidp_memory::context::issue_query_text(&mirror))
}

fn issue_route(ctx: &Ctx, project: &ProjectRef, issue: &str) -> Option<Route> {
    ticket_mirror(ctx, project, issue)
        .and_then(|mirror| aidp_memory::context::ticket_route(&mirror))
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
