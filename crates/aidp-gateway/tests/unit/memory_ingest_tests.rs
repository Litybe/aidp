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
