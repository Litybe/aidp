//! The daemon side of `aidp-memory`: the shared store, background re-indexing, the
//! ingestion of `memory-proposals.json` at the end of a run, and the bus listener that
//! triggers both. Failures here are logged, never raised to the run or the flow.

use crate::bus::Bus;
use crate::native_runs::{NativeRunManager, EVENT_RUN_ENDED};
use crate::projects::{Project, Projects};
use crate::sync::lock;
use aidp_memory::models::constants::limits::INGEST_MAX_DEPTH;
use aidp_memory::models::constants::paths::{NOTES_DIR, PROPOSALS_FILE};
use aidp_memory::models::constants::schema::GLOBAL_PROJECT;
use aidp_memory::models::ingest_report::IngestReport;
use aidp_memory::{
    IndexOptions, IndexReport, IndexRoot, Indexer, Memory, MemoryError, MemoryManager, MemoryStore,
    ProjectRef, Source,
};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Published by the flow executor after a successful `gh.merge`: `{ key, repo }`.
pub const EVENT_FLOW_MERGED: &str = "flow.merged";
/// A proposal reached the store (ingestion of a run, or `POST /api/memory`).
pub const EVENT_MEMORY_PROPOSED: &str = "memory.proposed";
/// A memory changed status or content.
pub const EVENT_MEMORY_CHANGED: &str = "memory.changed";
/// A re-index finished (or failed).
pub const EVENT_RAG_REINDEXED: &str = "rag.reindexed";

const MEMORY_DIR: &str = "memory";
const LEDGER_FILE: &str = "ingested.json";
const SCRATCH_DIR: &str = "ingest-tmp";
const LISTENER_THREAD: &str = "memory-listener";
const REINDEX_THREAD: &str = "memory-reindex";
const ISSUE_PREFIX: &str = "GH-";
const PIPELINES_PREFIX: &str = "Pipelines";
const TICKETS_DIR: &str = "TODO";
const KNOWLEDGE_PREFIX: &str = "Knowledge";
const TICKET_EXTENSION: &str = "md";
/// Sources a merge refreshes: the ones a merged change can alter.
pub const MERGE_SOURCES: [Source; 3] = [Source::Code, Source::Doc, Source::Bible];

/// One re-index to run in the background.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReindexJob {
    pub project: ProjectRef,
    /// Empty: every file source.
    pub sources: Vec<Source>,
    pub full: bool,
    /// Why it runs: `api` or `merge`.
    pub trigger: String,
}

/// What `enqueue_reindex` did with a job.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Enqueued {
    /// A worker thread was started for it.
    Started,
    /// A worker is running; the job waits behind it.
    Queued,
    /// An identical job already waits.
    Duplicate,
}

#[derive(Default)]
struct Queue {
    running: bool,
    jobs: VecDeque<ReindexJob>,
}

/// Facts about a finished run that complete the references of its proposals.
#[derive(Debug, Clone)]
pub struct RunFacts {
    pub run_id: String,
    pub run_dir: PathBuf,
    pub issue: Option<String>,
    pub pr: Option<u64>,
}

/// The shared memory store of the daemon, opened once, with its background work.
pub struct MemoryHub {
    state_dir: PathBuf,
    vault_root: PathBuf,
    bus: Arc<Bus>,
    store: Mutex<Option<Arc<MemoryStore>>>,
    queue: Mutex<Queue>,
    ledger: Mutex<()>,
}

impl MemoryHub {
    /// `state_dir` is the gateway directory (`memory/memory.db` lives under it).
    pub fn new(state_dir: &Path, vault_root: &Path, bus: Arc<Bus>) -> Arc<Self> {
        Arc::new(Self {
            state_dir: state_dir.to_path_buf(),
            vault_root: vault_root.to_path_buf(),
            bus,
            store: Mutex::new(None),
            queue: Mutex::new(Queue::default()),
            ledger: Mutex::new(()),
        })
    }

    /// The store, opened on first use and then shared (one writer connection).
    pub fn store(&self) -> Result<Arc<MemoryStore>, MemoryError> {
        let mut slot = lock(&self.store);
        if let Some(store) = slot.as_ref() {
            return Ok(store.clone());
        }
        let store = Arc::new(MemoryStore::open_in(&self.state_dir)?);
        *slot = Some(store.clone());
        Ok(store)
    }

    /// The project key (registered name) and root of a registered project.
    pub fn project_ref(project: &Project) -> ProjectRef {
        ProjectRef {
            key: project.name.clone(),
            root: PathBuf::from(&project.path),
        }
    }

    pub fn emit_changed(&self, memory: &Memory, action: &str) {
        self.bus.emit(
            EVENT_MEMORY_CHANGED,
            json!({
                "id": memory.id,
                "project": memory.project,
                "status": memory.status.as_str(),
                "action": action,
            }),
        );
    }

    pub fn emit_proposed(&self, memory: &Memory) {
        self.bus.emit(
            EVENT_MEMORY_PROPOSED,
            json!({ "project": memory.project, "ids": [memory.id], "created": 1 }),
        );
    }

    /// Directories outside the project that feed `run`, `ticket` and `knowledge`.
    pub fn extra_roots(&self, project: &ProjectRef) -> Vec<IndexRoot> {
        if self.vault_root.as_os_str().is_empty() {
            return Vec::new();
        }
        let repo = aidp_core::vault::repo_segment(&project.root);
        vec![
            IndexRoot {
                source: Source::Run,
                dir: self.vault_root.join(PIPELINES_PREFIX).join(&repo),
                prefix: format!("{PIPELINES_PREFIX}/{repo}"),
            },
            IndexRoot {
                source: Source::Ticket,
                dir: self.vault_root.join(TICKETS_DIR).join(&repo),
                prefix: format!("{TICKETS_DIR}/{repo}"),
            },
            IndexRoot {
                source: Source::Knowledge,
                dir: self.vault_root.join(KNOWLEDGE_PREFIX).join(&repo),
                prefix: format!("{KNOWLEDGE_PREFIX}/{repo}"),
            },
        ]
    }

    /// `<vault>/Knowledge/_global`, indexed as `knowledge` under the `_global` project.
    pub fn global_knowledge_root(&self) -> Option<IndexRoot> {
        if self.vault_root.as_os_str().is_empty() {
            return None;
        }
        Some(IndexRoot {
            source: Source::Knowledge,
            dir: self.vault_root.join(KNOWLEDGE_PREFIX).join(GLOBAL_PROJECT),
            prefix: format!("{KNOWLEDGE_PREFIX}/{GLOBAL_PROJECT}"),
        })
    }

    /// Brings the vault notes of a project in step with the database; failures are logged.
    pub fn sync_notes(&self, project: &ProjectRef) {
        if self.vault_root.as_os_str().is_empty() {
            return;
        }
        let dir = self
            .vault_root
            .join(KNOWLEDGE_PREFIX)
            .join(aidp_core::vault::repo_segment(&project.root))
            .join(NOTES_DIR);
        let outcome = self
            .store()
            .and_then(|store| MemoryManager::new(&store).sync_vault(&project.key, &dir));
        match outcome {
            Ok(report) => {
                for rejection in &report.rejected {
                    tracing::warn!(file = %rejection.file, "memory note refused: {}", rejection.reason);
                }
            }
            Err(error) => tracing::warn!(%error, "memory notes not synchronised"),
        }
    }

    /// True while a re-index runs or waits.
    pub fn reindexing(&self) -> bool {
        let queue = lock(&self.queue);
        queue.running || !queue.jobs.is_empty()
    }

    /// Queues a re-index; one worker thread drains the queue, one job at a time.
    pub fn enqueue_reindex(self: &Arc<Self>, job: ReindexJob) -> Enqueued {
        let mut queue = lock(&self.queue);
        if queue.jobs.contains(&job) {
            return Enqueued::Duplicate;
        }
        queue.jobs.push_back(job);
        if queue.running {
            return Enqueued::Queued;
        }
        queue.running = true;
        let hub = self.clone();
        let spawned = std::thread::Builder::new()
            .name(REINDEX_THREAD.to_string())
            .spawn(move || hub.drain_queue());
        if let Err(error) = spawned {
            tracing::warn!(%error, "memory re-index worker not started");
            queue.running = false;
            queue.jobs.clear();
        }
        Enqueued::Started
    }

    fn drain_queue(&self) {
        loop {
            let job = {
                let mut queue = lock(&self.queue);
                match queue.jobs.pop_front() {
                    Some(job) => job,
                    None => {
                        queue.running = false;
                        return;
                    }
                }
            };
            let outcome = crate::threads::guarded(REINDEX_THREAD, || self.run_reindex(&job));
            let payload = match outcome {
                Some(Ok(report)) => reindex_payload(&job, &report),
                Some(Err(error)) => failure_payload(&job, &error.to_string()),
                None => failure_payload(&job, "the re-index panicked"),
            };
            if payload["ok"] == Value::Bool(false) {
                tracing::warn!(project = %job.project.key, "memory re-index failed: {}", payload["error"]);
            }
            self.bus.emit(EVENT_RAG_REINDEXED, payload);
        }
    }

    fn run_reindex(&self, job: &ReindexJob) -> Result<IndexReport, MemoryError> {
        let store = self.store()?;
        let options = IndexOptions {
            sources: job.sources.clone(),
            full: job.full,
            extra_roots: self.extra_roots(&job.project),
            global_knowledge: self.global_knowledge_root(),
        };
        Indexer::new(&store).index(&job.project, &options)
    }

    /// Ingests every `memory-proposals.json` under the run folder that was not ingested yet,
    /// with the issue, pull request and run references completed from the run.
    pub fn ingest_run(
        &self,
        project: &ProjectRef,
        facts: &RunFacts,
    ) -> Result<IngestReport, MemoryError> {
        let store = self.store()?;
        let manager = MemoryManager::new(&store);
        let mut total = IngestReport::default();
        for file in proposal_files(&facts.run_dir) {
            let Ok(bytes) = std::fs::read(&file) else {
                continue;
            };
            let signature = signature_of(&file, &bytes);
            if self.already_ingested(&signature) {
                continue;
            }
            let repo = aidp_core::vault::repo_segment(&project.root);
            let report = self.ingest_file(&manager, &repo, &project.key, &bytes, facts)?;
            if report.rejected.is_empty() {
                self.mark_ingested(&signature);
            }
            total.files += report.files;
            total.created += report.created;
            total.deduplicated += report.deduplicated;
            total.activated += report.activated;
            total.rejected.extend(report.rejected);
        }
        if total.files > 0 {
            self.sync_notes(project);
            for rejection in &total.rejected {
                tracing::warn!(file = %rejection.file, "memory proposals refused: {}", rejection.reason);
            }
            if total.created + total.deduplicated > 0 {
                self.bus.emit(
                    EVENT_MEMORY_PROPOSED,
                    json!({
                        "project": project.key,
                        "runId": facts.run_id,
                        "files": total.files,
                        "created": total.created,
                        "deduplicated": total.deduplicated,
                        "activated": total.activated,
                        "rejected": total.rejected.len(),
                    }),
                );
            }
        }
        Ok(total)
    }

    fn ticket_route(&self, repo: &str, issue: &str) -> Option<aidp_memory::Route> {
        if self.vault_root.as_os_str().is_empty() {
            return None;
        }
        let mirror = std::fs::read_to_string(
            self.vault_root
                .join(TICKETS_DIR)
                .join(repo)
                .join(format!("{issue}.{TICKET_EXTENSION}")),
        )
        .ok()?;
        aidp_memory::context::ticket_route(&mirror)
    }

    fn ingest_file(
        &self,
        manager: &MemoryManager<'_>,
        repo: &str,
        project: &str,
        bytes: &[u8],
        facts: &RunFacts,
    ) -> Result<IngestReport, MemoryError> {
        let scratch = self
            .state_dir
            .join(MEMORY_DIR)
            .join(SCRATCH_DIR)
            .join(uuid::Uuid::new_v4().to_string());
        std::fs::create_dir_all(&scratch)?;
        let target = scratch.join(PROPOSALS_FILE);
        std::fs::write(&target, complete_refs(bytes, facts))?;
        let route_of = |issue: &str| self.ticket_route(repo, issue);
        let report = manager.ingest_routed(project, &target, &route_of);
        let _ = std::fs::remove_dir_all(&scratch);
        report
    }

    fn ledger_path(&self) -> PathBuf {
        self.state_dir.join(MEMORY_DIR).join(LEDGER_FILE)
    }

    fn read_ledger(&self) -> BTreeMap<String, bool> {
        std::fs::read_to_string(self.ledger_path())
            .ok()
            .and_then(|content| serde_json::from_str(&content).ok())
            .unwrap_or_default()
    }

    fn already_ingested(&self, signature: &str) -> bool {
        let _guard = lock(&self.ledger);
        self.read_ledger().contains_key(signature)
    }

    fn mark_ingested(&self, signature: &str) {
        let _guard = lock(&self.ledger);
        let mut ledger = self.read_ledger();
        ledger.insert(signature.to_string(), true);
        let path = self.ledger_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(error) = std::fs::write(&path, serde_json::to_vec(&ledger).unwrap_or_default()) {
            tracing::warn!(%error, "memory ingestion ledger not saved");
        }
    }

    /// Handles one bus event: `run.ended` ingests, `flow.merged` re-indexes.
    pub fn on_event(self: &Arc<Self>, event: &Value, runs: &NativeRunManager, projects: &Projects) {
        let data = &event["data"];
        match event["type"].as_str() {
            Some(EVENT_RUN_ENDED) => {
                if let Some(run_id) = data["runId"].as_str() {
                    self.on_run_ended(run_id, runs, projects);
                }
            }
            Some(EVENT_FLOW_MERGED) => {
                if let Some(repo) = data["repo"].as_str() {
                    self.on_merged(repo, projects);
                }
            }
            _ => {}
        }
    }

    fn on_run_ended(&self, run_id: &str, runs: &NativeRunManager, projects: &Projects) {
        let Some(view) = runs.get(run_id) else {
            return;
        };
        let project = project_for_workspace(projects, &view.workspace);
        let issue = view
            .request
            .ticket
            .clone()
            .or_else(|| view.request.target.clone())
            .map(|raw| normalize_issue(&raw));
        let facts = RunFacts {
            run_id: view.run_id.clone(),
            run_dir: view.run_dir.clone(),
            issue,
            pr: crate::pr_workspace_holder::pr_number_of(&view.request.inputs),
        };
        if let Err(error) = self.ingest_run(&project, &facts) {
            tracing::warn!(run = run_id, %error, "memory ingestion failed");
        }
        let staled = self
            .store()
            .and_then(|store| MemoryManager::new(&store).gc(&project.key));
        match staled {
            Ok(stale) if !stale.is_empty() => self.sync_notes(&project),
            Ok(_) => {}
            Err(error) => tracing::warn!(%error, "memory gc failed"),
        }
    }

    /// Re-indexes `code`, `doc` and `bible` of the project whose repository was merged.
    pub fn on_merged(self: &Arc<Self>, repo: &str, projects: &Projects) {
        let found = projects
            .list()
            .into_iter()
            .find(|project| aidp_core::vault::repo_segment(Path::new(&project.path)) == repo);
        let Some(project) = found else {
            return;
        };
        self.enqueue_reindex(ReindexJob {
            project: Self::project_ref(&project),
            sources: MERGE_SOURCES.to_vec(),
            full: false,
            trigger: "merge".to_string(),
        });
    }

    /// Starts the thread that turns `run.ended` and `flow.merged` into ingestion and re-indexing.
    pub fn spawn_listener(self: &Arc<Self>, runs: Arc<NativeRunManager>, projects: Arc<Projects>) {
        let events = self.bus.subscribe();
        let hub = self.clone();
        let spawned = std::thread::Builder::new()
            .name(LISTENER_THREAD.to_string())
            .spawn(move || {
                while let Ok(event) = events.recv() {
                    crate::threads::guarded(LISTENER_THREAD, || {
                        hub.on_event(&event, &runs, &projects)
                    });
                }
            });
        if let Err(error) = spawned {
            tracing::warn!(%error, "memory listener not started");
        }
    }
}

fn reindex_payload(job: &ReindexJob, report: &IndexReport) -> Value {
    json!({
        "ok": true,
        "project": job.project.key,
        "trigger": job.trigger,
        "indexed": report.indexed,
        "updated": report.updated,
        "removed": report.removed,
        "unchanged": report.unchanged,
        "chunks": report.chunks,
        "durationMs": report.duration_ms,
    })
}

fn failure_payload(job: &ReindexJob, error: &str) -> Value {
    json!({ "ok": false, "project": job.project.key, "trigger": job.trigger, "error": error })
}

fn proposal_files(run_dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = walkdir_files(run_dir);
    files.sort();
    files
}

/// Files named `memory-proposals.json` under `dir`, to the ingestion depth.
fn walkdir_files(dir: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() && depth < INGEST_MAX_DEPTH {
                walk(&path, depth + 1, out);
            } else if kind.is_file() && entry.file_name() == PROPOSALS_FILE {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, 1, &mut out);
    out
}

fn signature_of(path: &Path, bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    format!(
        "{}#{}",
        path.to_string_lossy().replace('\\', "/").to_lowercase(),
        digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    )
}

/// `214` and `#214` become `GH-214`; other keys are kept.
pub fn normalize_issue(raw: &str) -> String {
    let trimmed = raw.trim().trim_start_matches('#');
    if !trimmed.is_empty() && trimmed.chars().all(|c| c.is_ascii_digit()) {
        return format!("{ISSUE_PREFIX}{trimmed}");
    }
    trimmed.to_string()
}

/// Adds the issue, pull request and run references the proposals lack. An unparsable
/// document is returned unchanged: ingestion then refuses it with its own message.
fn complete_refs(bytes: &[u8], facts: &RunFacts) -> Vec<u8> {
    let Ok(mut document) = serde_json::from_slice::<Value>(bytes) else {
        return bytes.to_vec();
    };
    let mut wanted: Vec<(&str, String)> = Vec::new();
    if let Some(issue) = &facts.issue {
        wanted.push(("issue", issue.clone()));
    }
    if let Some(pr) = facts.pr {
        wanted.push(("pr", pr.to_string()));
    }
    wanted.push(("run", facts.run_id.clone()));
    if let Some(proposals) = document.get_mut("proposals").and_then(Value::as_array_mut) {
        for proposal in proposals {
            let Some(object) = proposal.as_object_mut() else {
                continue;
            };
            let refs = object
                .entry("refs")
                .or_insert_with(|| Value::Array(Vec::new()));
            let Some(list) = refs.as_array_mut() else {
                continue;
            };
            for (kind, id) in &wanted {
                let present = list
                    .iter()
                    .any(|entry| entry["type"] == *kind && entry["id"] == *id);
                if !present {
                    list.push(json!({ "type": kind, "id": id }));
                }
            }
        }
    }
    serde_json::to_vec(&document).unwrap_or_else(|_| bytes.to_vec())
}

fn comparable(path: &str) -> String {
    path.replace('\\', "/").trim_end_matches('/').to_lowercase()
}

/// The registered project of a workspace, else a key made from its folder name.
fn project_for_workspace(projects: &Projects, workspace: &Path) -> ProjectRef {
    let wanted = comparable(&workspace.to_string_lossy());
    let list = projects.list();
    let found = list
        .iter()
        .find(|project| comparable(&project.path) == wanted)
        .or_else(|| {
            let segment = aidp_core::vault::repo_segment(workspace);
            list.iter()
                .find(|project| aidp_core::vault::repo_segment(Path::new(&project.path)) == segment)
        });
    match found {
        Some(project) => MemoryHub::project_ref(project),
        None => {
            let anchor = aidp_core::vault::main_worktree_root(workspace);
            ProjectRef {
                key: anchor
                    .file_name()
                    .map_or_else(|| wanted.clone(), |name| name.to_string_lossy().to_string()),
                root: anchor,
            }
        }
    }
}

#[cfg(test)]
#[path = "../tests/unit/memory_ingest_tests.rs"]
mod tests;
