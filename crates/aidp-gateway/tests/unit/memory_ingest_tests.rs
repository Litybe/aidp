use super::*;
use aidp_testkit::TempWorkspace;
use std::fs;

const PROPOSALS: &str = r#"{ "proposals": [
  { "kind": "lesson", "title": "Rigidbody in FixedUpdate", "text": "Set velocity in FixedUpdate.",
    "refs": [ { "type": "issue", "id": "GH-1" } ], "confidence": 0.7 } ] }"#;

const LEGACY_PROPOSALS: &str = r#"{ "proposals": [ { "type": "feedback", "title": "Meta copie", "body": "Verifier le nom." } ] }"#;

fn hub(state: &TempWorkspace) -> Arc<MemoryHub> {
    MemoryHub::new(state.path(), Path::new(""), Arc::new(Bus::new()))
}

fn project() -> ProjectRef {
    ProjectRef {
        key: "demo".to_string(),
        root: PathBuf::from("."),
    }
}

fn facts(run_dir: &Path) -> RunFacts {
    RunFacts {
        run_id: "run-1".to_string(),
        run_dir: run_dir.to_path_buf(),
        issue: Some("GH-7".to_string()),
        pr: Some(12),
    }
}

fn write_proposals(run: &TempWorkspace, stage: &str, content: &str) {
    let dir = run.path().join(stage);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(PROPOSALS_FILE), content).unwrap();
}

#[test]
fn ingestion_completes_refs_and_is_idempotent() {
    let state = TempWorkspace::new("mem-state");
    let run = TempWorkspace::new("mem-run");
    write_proposals(&run, "03-review", PROPOSALS);
    let hub = hub(&state);
    let events = hub.bus.subscribe();

    let first = hub.ingest_run(&project(), &facts(run.path())).unwrap();
    assert_eq!((first.files, first.created), (1, 1));
    let again = hub.ingest_run(&project(), &facts(run.path())).unwrap();
    assert_eq!(again.files, 0);

    let store = hub.store().unwrap();
    let all = MemoryManager::new(&store)
        .list("demo", &Default::default())
        .unwrap();
    assert_eq!(all.len(), 1);
    let kinds: Vec<(String, String)> = all[0]
        .refs
        .iter()
        .map(|r| (r.ref_type.as_str().to_string(), r.id.clone()))
        .collect();
    for wanted in [
        ("issue", "GH-1"),
        ("issue", "GH-7"),
        ("pr", "12"),
        ("run", "run-1"),
    ] {
        assert!(
            kinds.contains(&(wanted.0.to_string(), wanted.1.to_string())),
            "{wanted:?}"
        );
    }
    let event = events.try_recv().unwrap();
    assert_eq!(event["type"], EVENT_MEMORY_PROPOSED);
    assert_eq!(event["data"]["created"], 1);
    assert!(events.try_recv().is_err(), "one event for one ingestion");
}

#[test]
fn a_refused_file_is_reported_not_raised_and_retried() {
    let state = TempWorkspace::new("mem-state");
    let run = TempWorkspace::new("mem-run");
    write_proposals(&run, "a", "{ not json");
    write_proposals(&run, "b", PROPOSALS);
    let hub = hub(&state);
    let report = hub.ingest_run(&project(), &facts(run.path())).unwrap();
    assert_eq!(
        (report.files, report.created, report.rejected.len()),
        (2, 1, 1)
    );
    let retry = hub.ingest_run(&project(), &facts(run.path())).unwrap();
    assert_eq!(
        (retry.files, retry.created, retry.rejected.len()),
        (1, 0, 1)
    );
}

#[test]
fn an_undocumented_shape_reaches_the_store_with_the_run_refs() {
    let state = TempWorkspace::new("mem-state");
    let run = TempWorkspace::new("mem-run");
    write_proposals(&run, "a", LEGACY_PROPOSALS);
    let hub = hub(&state);
    let report = hub.ingest_run(&project(), &facts(run.path())).unwrap();
    assert_eq!((report.created, report.rejected.len()), (1, 0));
    let store = hub.store().unwrap();
    let memory = MemoryManager::new(&store)
        .list("demo", &Default::default())
        .unwrap()
        .remove(0);
    assert_eq!(memory.refs.len(), 3);
}

#[test]
fn a_changed_file_is_ingested_again() {
    let state = TempWorkspace::new("mem-state");
    let run = TempWorkspace::new("mem-run");
    write_proposals(&run, "a", PROPOSALS);
    let hub = hub(&state);
    hub.ingest_run(&project(), &facts(run.path())).unwrap();
    write_proposals(
        &run,
        "a",
        &PROPOSALS.replace("Rigidbody in FixedUpdate", "Cache GetComponent in Awake"),
    );
    assert_eq!(
        hub.ingest_run(&project(), &facts(run.path()))
            .unwrap()
            .files,
        1
    );
}

#[test]
fn issue_numbers_are_normalized() {
    assert_eq!(normalize_issue("214"), "GH-214");
    assert_eq!(normalize_issue("#214"), "GH-214");
    assert_eq!(normalize_issue("GH-214"), "GH-214");
}

fn mini_project() -> TempWorkspace {
    let project = TempWorkspace::new("mem-proj");
    let docs = project.path().join("Docs");
    fs::create_dir_all(&docs).unwrap();
    fs::write(
        docs.join("guide.md"),
        "# Guide\n\nThe dash needs a cooldown.\n",
    )
    .unwrap();
    project
}

fn wait_reindexed(events: &std::sync::mpsc::Receiver<Value>) -> Value {
    loop {
        let event = events
            .recv_timeout(std::time::Duration::from_secs(30))
            .expect("rag.reindexed");
        if event["type"] == EVENT_RAG_REINDEXED {
            return event["data"].clone();
        }
    }
}

#[test]
fn reindex_runs_in_the_background_and_publishes_counters() {
    let state = TempWorkspace::new("mem-state");
    let project = mini_project();
    let hub = hub(&state);
    let events = hub.bus.subscribe();
    let job = ReindexJob {
        project: ProjectRef {
            key: "demo".to_string(),
            root: project.path().to_path_buf(),
        },
        sources: vec![Source::Doc],
        full: false,
        trigger: "api".to_string(),
    };
    assert_eq!(hub.enqueue_reindex(job), Enqueued::Started);
    let data = wait_reindexed(&events);
    assert_eq!(data["ok"], true);
    assert_eq!(data["indexed"], 1);
    assert!(data["chunks"].as_u64().unwrap() >= 1);
}

#[test]
fn a_merge_reindexes_the_registered_project() {
    let state = TempWorkspace::new("mem-state");
    let project = mini_project();
    let projects = Projects::load(state.path());
    projects.add(project.path().to_str().unwrap());
    let hub = hub(&state);
    let events = hub.bus.subscribe();
    let repo = aidp_core::vault::repo_segment(project.path());
    hub.on_merged("someone-else", &projects);
    assert!(!hub.reindexing());
    hub.on_merged(&repo, &projects);
    let data = wait_reindexed(&events);
    assert_eq!(data["trigger"], "merge");
    assert_eq!(data["indexed"], 1);
}

const KNOWLEDGE_PROPOSALS: &str = r#"{ "proposals": [
  { "action": "add", "domain": "unity", "scope": "project", "slug": "fixed-update-physics",
    "title": "Physique en FixedUpdate", "body": "Les forces du rigidbody sappliquent dans FixedUpdate.",
    "tags": ["physics"], "evidence": "review du run" } ] }"#;
const FAKE_MODEL: &str = "fake";
const FAKE_DIM: usize = 4;

struct ConstantEmbedder;

impl Embedder for ConstantEmbedder {
    fn model_id(&self) -> &str {
        FAKE_MODEL
    }

    fn dim(&self) -> usize {
        FAKE_DIM
    }

    fn embed(&self, texts: &[&str]) -> aidp_memory::Result<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|_| vec![1.0; FAKE_DIM]).collect())
    }
}

fn vault_hub(state: &TempWorkspace, vault: &TempWorkspace, embedded: bool) -> Arc<MemoryHub> {
    let embedder: Option<Arc<dyn Embedder>> = embedded.then(|| Arc::new(ConstantEmbedder) as _);
    MemoryHub::with_embedder(state.path(), vault.path(), Arc::new(Bus::new()), embedder)
}

fn write_knowledge(run: &TempWorkspace, stage: &str, content: &str) {
    let dir = run.path().join(stage);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(KNOWLEDGE_PROPOSALS_FILE), content).unwrap();
}

fn project_in(workspace: &TempWorkspace) -> ProjectRef {
    ProjectRef {
        key: "demo".to_string(),
        root: workspace.path().to_path_buf(),
    }
}

fn note_path(vault: &TempWorkspace, project: &ProjectRef) -> PathBuf {
    vault
        .path()
        .join("Knowledge")
        .join(aidp_core::vault::repo_segment(&project.root))
        .join("notes/unity/fixed-update-physics.md")
}

#[test]
fn knowledge_proposals_are_ingested_once_with_the_run_as_source() {
    let state = TempWorkspace::new("mem-state");
    let vault = TempWorkspace::new("mem-vault");
    let workspace = mini_project();
    let run = TempWorkspace::new("mem-run");
    write_knowledge(&run, "05-learn", KNOWLEDGE_PROPOSALS);
    let hub = vault_hub(&state, &vault, false);
    let events = hub.bus.subscribe();
    let project = project_in(&workspace);

    let first = hub.ingest_knowledge(&project, &facts(run.path())).unwrap();
    assert_eq!((first.files, first.added), (1, 1));
    let note = fs::read_to_string(note_path(&vault, &project)).unwrap();
    assert!(note.contains("source: run:run-1\n"), "{note}");
    assert!(note.contains("tags: [physics]\n"));
    let again = hub.ingest_knowledge(&project, &facts(run.path())).unwrap();
    assert_eq!(again.files, 0);
    let event = events
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    assert_eq!(event["type"], EVENT_KNOWLEDGE_CHANGED);
    assert_eq!(event["data"]["added"], 1);
}

#[test]
fn ingest_run_handles_memory_and_knowledge_files_together() {
    let state = TempWorkspace::new("mem-state");
    let vault = TempWorkspace::new("mem-vault");
    let workspace = mini_project();
    let run = TempWorkspace::new("mem-run");
    write_proposals(&run, "03-review", PROPOSALS);
    write_knowledge(&run, "05-learn", KNOWLEDGE_PROPOSALS);
    let hub = vault_hub(&state, &vault, false);
    let project = project_in(&workspace);

    let memory = hub.ingest_run(&project, &facts(run.path())).unwrap();
    assert_eq!(memory.files, 1);
    assert!(note_path(&vault, &project).is_file());
}

#[test]
fn a_hub_without_a_vault_leaves_knowledge_proposals_alone() {
    let state = TempWorkspace::new("mem-state");
    let run = TempWorkspace::new("mem-run");
    write_knowledge(&run, "05-learn", KNOWLEDGE_PROPOSALS);
    let report = hub(&state)
        .ingest_knowledge(&project(), &facts(run.path()))
        .unwrap();
    assert_eq!(report.files, 0);
}

#[test]
fn a_refused_knowledge_file_is_reported_and_not_retried() {
    let state = TempWorkspace::new("mem-state");
    let vault = TempWorkspace::new("mem-vault");
    let workspace = mini_project();
    let run = TempWorkspace::new("mem-run");
    write_knowledge(
        &run,
        "05-learn",
        r#"{ "proposals": [ { "action": "add" } ] }"#,
    );
    let hub = vault_hub(&state, &vault, false);
    let project = project_in(&workspace);
    let first = hub.ingest_knowledge(&project, &facts(run.path())).unwrap();
    assert_eq!((first.files, first.rejected.len()), (0, 1));
    let again = hub.ingest_knowledge(&project, &facts(run.path())).unwrap();
    assert!(again.rejected.is_empty());
    assert!(!vault.path().join("Knowledge").exists());
}

fn wait_trigger(events: &std::sync::mpsc::Receiver<Value>, trigger: &str) -> Value {
    loop {
        let data = wait_reindexed(events);
        if data["trigger"] == trigger {
            return data;
        }
    }
}

fn count(hub: &MemoryHub, sql: &str) -> i64 {
    hub.store()
        .unwrap()
        .read(|connection| Ok(connection.query_row(sql, [], |row| row.get(0))?))
        .unwrap()
}

#[test]
fn a_finished_run_ingests_embeds_memories_and_reindexes_its_sources() {
    let state = TempWorkspace::new("mem-state");
    let vault = TempWorkspace::new("mem-vault");
    let workspace = mini_project();
    let run = TempWorkspace::new("mem-run");
    write_proposals(&run, "03-review", PROPOSALS);
    write_knowledge(&run, "05-learn", KNOWLEDGE_PROPOSALS);
    let hub = vault_hub(&state, &vault, true);
    let events = hub.bus.subscribe();
    let project = project_in(&workspace);

    hub.finish_run(project.clone(), &facts(run.path()));

    let memory_vectors = count(
        &hub,
        "SELECT COUNT(*) FROM chunk_vectors v JOIN chunks c ON c.rowid = v.chunk_id WHERE c.source = 'memory'",
    );
    assert_eq!(
        memory_vectors, 1,
        "memories are embedded without a re-index"
    );
    let data = wait_trigger(&events, "run");
    assert_eq!(data["ok"], true);
    let note_chunks = count(
        &hub,
        "SELECT COUNT(*) FROM chunks WHERE source = 'knowledge'",
    );
    assert!(note_chunks >= 1, "the new note is indexed");
    let note_vectors = count(
        &hub,
        "SELECT COUNT(*) FROM chunk_vectors v JOIN chunks c ON c.rowid = v.chunk_id WHERE c.source = 'knowledge'",
    );
    assert_eq!(note_vectors, note_chunks);
}

fn write_native_run(
    vault: &TempWorkspace,
    repo: &str,
    name: &str,
    ended: Option<String>,
) -> PathBuf {
    let dir = vault
        .path()
        .join("Pipelines")
        .join(repo)
        .join("feature")
        .join(name);
    fs::create_dir_all(&dir).unwrap();
    let view = json!({
        "runId": name, "templateId": "dev", "status": "done",
        "workspace": dir.to_string_lossy(), "runDir": dir.to_string_lossy(),
        "request": { "projectId": "p", "templateId": "dev", "ticket": "GH-7" },
        "startedAt": "2026-01-01T00:00:00+00:00", "endedAt": ended,
        "plan": [], "stages": []
    });
    fs::write(dir.join("native-run.json"), view.to_string()).unwrap();
    dir
}

#[test]
fn catch_up_ingests_recent_ended_runs_and_reindexes_once() {
    let state = TempWorkspace::new("mem-state");
    let vault = TempWorkspace::new("mem-vault");
    let workspace = mini_project();
    let projects = Projects::load(state.path());
    projects.add(workspace.path().to_str().unwrap());
    let repo = aidp_core::vault::repo_segment(workspace.path());
    let recent = (Utc::now() - Duration::days(2)).to_rfc3339();
    let stale = (Utc::now() - Duration::days(40)).to_rfc3339();
    let fresh_dir = write_native_run(&vault, &repo, "run-recent", Some(recent));
    let old_dir = write_native_run(&vault, &repo, "run-old", Some(stale));
    let live_dir = write_native_run(&vault, &repo, "run-live", None);
    for (dir, slug) in [
        (&fresh_dir, "recent-note"),
        (&old_dir, "old-note"),
        (&live_dir, "live-note"),
    ] {
        fs::write(
            dir.join(KNOWLEDGE_PROPOSALS_FILE),
            KNOWLEDGE_PROPOSALS.replace("fixed-update-physics", slug),
        )
        .unwrap();
    }
    let hub = vault_hub(&state, &vault, false);
    let events = hub.bus.subscribe();

    let report = hub.catch_up(&projects);

    assert_eq!(
        (report.projects, report.runs, report.knowledge_files),
        (1, 1, 1)
    );
    let notes = vault
        .path()
        .join("Knowledge")
        .join(&repo)
        .join("notes/unity");
    assert!(notes.join("recent-note.md").is_file());
    assert!(!notes.join("old-note.md").exists());
    assert!(!notes.join("live-note.md").exists());
    let data = wait_trigger(&events, "catch-up");
    assert_eq!(data["ok"], true);
    let second = hub.catch_up(&projects);
    assert_eq!(second.knowledge_files, 0, "the ledger remembers the file");
}

#[test]
fn catch_up_without_a_vault_does_nothing() {
    let state = TempWorkspace::new("mem-state");
    let projects = Projects::load(state.path());
    let report = hub(&state).catch_up(&projects);
    assert_eq!(report, MemoryCatchUpReport::default());
}

#[test]
fn the_hub_retriever_follows_the_embedder_and_the_lexical_flag() {
    let state = TempWorkspace::new("mem-state");
    let vault = TempWorkspace::new("mem-vault");
    let workspace = mini_project();
    let hub = vault_hub(&state, &vault, true);
    let store = hub.store().unwrap();
    hub.enqueue_reindex(ReindexJob {
        project: project_in(&workspace),
        sources: vec![Source::Doc],
        full: false,
        trigger: "api".to_string(),
    });
    while hub.reindexing() {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let query = aidp_memory::SearchQuery::new("demo", "dash cooldown");
    let hybrid = hub.retriever(&store, None, false).search(&query).unwrap();
    let lexical = hub.retriever(&store, None, true).search(&query).unwrap();
    assert!(hybrid.iter().any(|hit| hit.via == "hybrid"), "{hybrid:?}");
    assert!(lexical.iter().all(|hit| hit.via == "bm25"));
}

#[test]
fn concurrent_ingestions_of_one_run_ingest_it_once() {
    let state = TempWorkspace::new("mem-state");
    let vault = TempWorkspace::new("mem-vault");
    let workspace = mini_project();
    let run = TempWorkspace::new("mem-run");
    write_knowledge(&run, "05-learn", KNOWLEDGE_PROPOSALS);
    let hub = vault_hub(&state, &vault, false);
    let project = project_in(&workspace);
    let run_facts = facts(run.path());
    let files: usize = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..8)
            .map(|_| scope.spawn(|| hub.ingest_knowledge(&project, &run_facts).unwrap().files))
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .sum()
    });
    assert_eq!(files, 1);
}

#[test]
fn an_issue_key_that_climbs_out_of_the_tickets_folder_has_no_route() {
    let state = TempWorkspace::new("mem-state");
    let vault = TempWorkspace::new("mem-vault");
    let hub = vault_hub(&state, &vault, false);
    let tickets = vault.path().join(TICKETS_DIR);
    fs::create_dir_all(tickets.join("repo")).unwrap();
    fs::write(tickets.join("repo/ok.md"), "---\nroute: code\n---\n").unwrap();
    fs::write(tickets.join("secret.md"), "---\nroute: art\n---\n").unwrap();
    assert!(hub.ticket_route("repo", "ok").is_some());
    assert!(hub.ticket_route("repo", "../secret").is_none());
}
