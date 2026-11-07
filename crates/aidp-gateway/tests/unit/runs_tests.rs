use super::*;
use aidp_testkit::{EnvGuard, TempWorkspace};
use std::fs;

const MATCH_BY_RUN_ID_WIRE: &str = "run_id";

struct WriteRun<'a> {
    repo: &'a str,
    feature: &'a str,
    run_id: &'a str,
    workspace_root: &'a str,
    started_at: &'a str,
    workflow_run_id: Option<&'a str>,
    extra: Option<Value>,
}

impl<'a> Default for WriteRun<'a> {
    fn default() -> Self {
        Self {
            repo: "",
            feature: "feature-1",
            run_id: "",
            workspace_root: "",
            started_at: "",
            workflow_run_id: None,
            extra: None,
        }
    }
}

fn write_run(vault: &Path, args: WriteRun) {
    let dir = vault
        .join("Pipelines")
        .join(args.repo)
        .join(args.feature)
        .join(args.run_id);
    fs::create_dir_all(&dir).unwrap();
    let mut v = json!({
        "run_id": args.run_id,
        "pipeline_path": "templates/dev-aidp.yaml",
        "pipeline_id": "dev-aidp",
        "workspace_root": args.workspace_root,
        "run_dir": dir.to_string_lossy(),
        "started_at": args.started_at,
        "repo": args.repo,
        "feature": args.feature,
        "vault_root": vault.to_string_lossy(),
        "stages": [],
    });
    if let Some(w) = args.workflow_run_id {
        v["workflow_run_id"] = json!(w);
    }
    if let Some(extra) = args.extra {
        if let (Value::Object(base), Value::Object(over)) = (&mut v, &extra) {
            for (k, val) in over {
                base.insert(k.clone(), val.clone());
            }
        }
    }
    fs::write(
        dir.join("run-state.json"),
        serde_json::to_string(&v).unwrap(),
    )
    .unwrap();
}

#[test]
fn env_dependent_cases() {
    let workspace = TempWorkspace::new("test-vault");
    let vault = workspace.path().to_path_buf();
    let _env_guard = EnvGuard::set("AIDP_VAULT", vault.to_string_lossy().as_ref());

    write_run(
        &vault,
        WriteRun {
            repo: "repo-a",
            run_id: "run-1",
            workspace_root: "C:/code/repo-a",
            started_at: "2026-01-01T10:00:00Z",
            workflow_run_id: Some("wf-a"),
            ..Default::default()
        },
    );
    let found = resolve_run("C:/code/repo-a", Some("wf-a")).unwrap();
    assert_eq!(found.run_id, "run-1");
    assert!(resolve_run("C:/code/repo-a", None).is_none());

    // No candidate at all -> None.
    assert!(resolve_run("C:/code/nowhere", Some("wf-a")).is_none());

    // workspace_root differing only by separators and case -> matched.
    write_run(
        &vault,
        WriteRun {
            repo: "repo-c",
            run_id: "run-c",
            workspace_root: "C:\\Code\\Repo-C\\",
            started_at: "2026-03-01T00:00:00Z",
            workflow_run_id: Some("wf-c"),
            ..Default::default()
        },
    );
    let found = resolve_run("c:/code/repo-c", Some("wf-c")).unwrap();
    assert_eq!(found.run_id, "run-c");

    // workflow_run_id wins outright when present.
    write_run(
        &vault,
        WriteRun {
            repo: "repo-d",
            run_id: "run-d1",
            workspace_root: "C:/code/repo-d",
            started_at: "2026-04-01T00:00:00Z",
            workflow_run_id: Some("wf-1"),
            ..Default::default()
        },
    );
    write_run(
        &vault,
        WriteRun {
            repo: "repo-d",
            run_id: "run-d2",
            workspace_root: "C:/code/repo-d",
            started_at: "2026-04-01T00:00:01Z",
            workflow_run_id: Some("wf-2"),
            ..Default::default()
        },
    );
    let found = resolve_run("C:/code/repo-d", Some("wf-2")).unwrap();
    assert_eq!(found.run_id, "run-d2");

    // A pending_gate run normalises to status "paused" with a non-null currentGate.
    write_run(
        &vault,
        WriteRun {
            repo: "repo-e",
            run_id: "run-e",
            workspace_root: "C:/code/repo-e",
            started_at: "2026-05-01T00:00:00Z",
            extra: Some(json!({ "pending_gate": "review", "stages": [
                { "name": "plan", "status": "success", "started_at": "2026-05-01T00:00:00Z", "finished_at": "2026-05-01T00:01:00Z", "duration_ms": 60000, "tokens_used": 10, "stage_dir": "x" }
            ] })),
            ..Default::default()
        },
    );
    let run_dir = vault
        .join("Pipelines")
        .join("repo-e")
        .join("feature-1")
        .join("run-e");
    let normalized = normalize(&run_dir).unwrap();
    assert_eq!(normalized["status"], json!("paused"));
    assert_eq!(
        normalized["currentGate"],
        json!({ "stage": "review", "anchorPersisted": true })
    );
    assert!(!normalized["currentGate"].is_null());

    let for_sess = for_session_with_run_id(
        "sess-1",
        "C:/code/repo-a",
        "2026-01-01T10:00:00Z",
        Some("wf-a"),
    )
    .unwrap();
    assert_eq!(for_sess["sessionId"], json!("sess-1"));
    assert_eq!(for_sess["match"]["by"], json!(MATCH_BY_RUN_ID_WIRE));
    assert!(
        for_session_with_run_id("sess-2", "C:/code/repo-a", "2026-01-01T10:00:00Z", None).is_none()
    );
}

#[test]
fn resolve_run_bounds_scan_to_recent_runs_per_feature_dir() {
    let workspace = TempWorkspace::new("bound-test");
    let vault = workspace.path().to_path_buf();
    let _env_guard = EnvGuard::set("AIDP_VAULT", vault.to_string_lossy().as_ref());

    // Seed far more runs in one feature dir than MAX_RUNS_PER_FEATURE_DIR,
    // all far too old to match, plus one recent run that should still be
    // found because run ids are timestamp-prefixed and lexical descending
    // order visits it first.
    for i in 0..(MAX_RUNS_PER_FEATURE_DIR + 50) {
        write_run(
            &vault,
            WriteRun {
                repo: "repo-bound",
                run_id: &format!("20200101-{i:06}-old"),
                workspace_root: "C:/code/repo-bound",
                started_at: "2020-01-01T00:00:00Z",
                ..Default::default()
            },
        );
    }
    write_run(
        &vault,
        WriteRun {
            repo: "repo-bound",
            run_id: "20260910-162533-recent",
            workspace_root: "C:/code/repo-bound",
            started_at: "2026-09-10T16:25:33Z",
            workflow_run_id: Some("wf-recent"),
            ..Default::default()
        },
    );

    let found = resolve_run("C:/code/repo-bound", Some("wf-recent")).unwrap();
    assert_eq!(found.run_id, "20260910-162533-recent");
}

#[test]
fn same_path_normalises_separators_and_case() {
    assert!(same_path("C:\\Code\\Repo", "c:/code/repo"));
    assert!(same_path("C:/code/repo/", "C:/code/repo"));
    assert!(!same_path("C:/code/repo", "C:/code/other"));
}

#[test]
fn stage_status_maps_every_value() {
    assert_eq!(stage_status(StageStatus::Pending).as_str(), "pending");
    assert_eq!(stage_status(StageStatus::InProgress).as_str(), "running");
    assert_eq!(stage_status(StageStatus::Success).as_str(), "done");
    assert_eq!(stage_status(StageStatus::Cancel).as_str(), "cancelled");
    assert_eq!(stage_status(StageStatus::Skipped).as_str(), "skipped");
}

#[test]
fn build_context_block_renders_summaries_and_documents() {
    let a = ChainArtefacts {
        run_id: "run-1".to_string(),
        feature: "feature-x".to_string(),
        pipeline_id: "dev-aidp".to_string(),
        stage_summaries: vec![("plan".to_string(), "Planned the thing.".to_string())],
        documents: vec![(
            "plan/plan.md".to_string(),
            "# Plan\nDo the thing.".to_string(),
        )],
    };
    let block = build_context_block(&a);
    assert!(block.contains("## Contexte du run précédent"));
    assert!(block.contains("Run `run-1` — pipeline `dev-aidp`, feature `feature-x`."));
    assert!(block.contains("- **plan** — Planned the thing."));
    assert!(block.contains("### plan/plan.md"));
    assert!(block.contains("Do the thing."));
}

#[test]
fn build_context_block_is_empty_without_content() {
    let a = ChainArtefacts::default();
    assert_eq!(build_context_block(&a), "");
}

#[test]
fn context_block_truncates_oversized_documents() {
    let big = "a".repeat(20_000);
    let a = ChainArtefacts {
        run_id: "run-1".to_string(),
        feature: "feature-x".to_string(),
        pipeline_id: "dev-aidp".to_string(),
        stage_summaries: vec![],
        documents: vec![("plan/plan.md".to_string(), big)],
    };
    let block = build_context_block(&a);
    assert!(block.contains("[... tronqué]"));
    assert!(block.chars().count() <= CHAIN_BLOCK_MAX_CHARS + 32);
}

#[test]
fn truncate_is_char_safe_on_multibyte() {
    let s: String = "é".repeat(100);
    let truncated = truncate_chars(&s, 40);
    assert!(truncated.starts_with(&"é".repeat(40)));
    assert!(truncated.ends_with("[... tronqué]"));
    assert_eq!(truncated.chars().take(40).count(), 40);
}

#[test]
fn read_chain_artefacts_collects_allowlisted_files() {
    let workspace = TempWorkspace::new("chain-test");
    let run_dir = workspace.child("run-x");
    let stage_dir = run_dir.join("plan");
    fs::create_dir_all(&stage_dir).unwrap();
    fs::write(stage_dir.join("plan.md"), "# The plan").unwrap();
    fs::write(stage_dir.join("notes.md"), "not allowlisted").unwrap();

    let v = json!({
        "run_id": "run-x",
        "pipeline_path": "templates/dev-aidp.yaml",
        "pipeline_id": "dev-aidp",
        "run_dir": "run-x",
        "workspace_root": "C:/code/repo-x",
        "started_at": "2026-06-01T00:00:00Z",
        "repo": "repo-x",
        "feature": "feature-x",
        "stages": [
            { "name": "plan", "status": "success", "summary": "Wrote the plan.", "stage_dir": "plan" }
        ],
    });
    fs::write(
        run_dir.join("run-state.json"),
        serde_json::to_string(&v).unwrap(),
    )
    .unwrap();

    let artefacts = read_chain_artefacts(&run_dir).unwrap();
    assert_eq!(artefacts.run_id, "run-x");
    assert_eq!(
        artefacts.stage_summaries,
        vec![("plan".to_string(), "Wrote the plan.".to_string())]
    );
    assert_eq!(artefacts.documents.len(), 1);
    assert_eq!(artefacts.documents[0].0, "plan/plan.md");
    assert!(artefacts.documents[0].1.contains("The plan"));
}

#[test]
fn read_chain_artefacts_bounds_oversized_file_reads() {
    let workspace = TempWorkspace::new("chain-bigfile-test");
    let run_dir = workspace.child("run-big");
    let stage_dir = run_dir.join("plan");
    fs::create_dir_all(&stage_dir).unwrap();
    // Far bigger than CHAIN_DOC_MAX_CHARS * 4 (the read_bounded budget).
    let huge = "a".repeat(CHAIN_DOC_MAX_CHARS * 4 * 3);
    fs::write(stage_dir.join("plan.md"), &huge).unwrap();

    let v = json!({
        "run_id": "run-big",
        "pipeline_path": "templates/dev-aidp.yaml",
        "pipeline_id": "dev-aidp",
        "run_dir": "run-big",
        "workspace_root": "C:/code/repo-big",
        "started_at": "2026-06-01T00:00:00Z",
        "repo": "repo-big",
        "feature": "feature-big",
        "stages": [
            { "name": "plan", "status": "success", "stage_dir": "plan" }
        ],
    });
    fs::write(
        run_dir.join("run-state.json"),
        serde_json::to_string(&v).unwrap(),
    )
    .unwrap();

    let artefacts = read_chain_artefacts(&run_dir).unwrap();
    assert_eq!(artefacts.documents.len(), 1);
    // What's read off disk (pre-truncate_chars) is bounded well below
    // the full file size, proving the file wasn't read whole.
    assert!(artefacts.documents[0].1.len() <= CHAIN_DOC_MAX_CHARS * 4);
    assert!(artefacts.documents[0].1.len() < huge.len());
}

fn bare_run_state(extra: Value) -> Value {
    let mut state = json!({
        "run_id": "run-anchor",
        "pipeline_path": "templates/dev-aidp.yaml",
        "pipeline_id": "dev-aidp",
        "workspace_root": "C:/code/repo-bare",
        "run_dir": "run-anchor",
        "started_at": "2026-06-01T00:00:00Z",
        "stages": [],
    });
    if let (Value::Object(base), Value::Object(over)) = (&mut state, &extra) {
        for (key, value) in over {
            base.insert(key.clone(), value.clone());
        }
    }
    state
}

#[test]
fn run_is_gate_blocked_detects_pending_gate() {
    let workspace = TempWorkspace::new("gate-test");
    let run_dir = workspace.child("run-gate");
    fs::create_dir_all(&run_dir).unwrap();
    fs::write(
        run_dir.join("run-state.json"),
        serde_json::to_string(&bare_run_state(json!({ "pending_gate": "review" }))).unwrap(),
    )
    .unwrap();
    assert!(run_is_gate_blocked(&run_dir));

    let run_dir_clean = workspace.child("run-clean");
    fs::create_dir_all(&run_dir_clean).unwrap();
    fs::write(
        run_dir_clean.join("run-state.json"),
        serde_json::to_string(&bare_run_state(json!({ "pending_gate": "" }))).unwrap(),
    )
    .unwrap();
    assert!(!run_is_gate_blocked(&run_dir_clean));
}

#[test]
fn pending_gate_stage_agrees_with_run_is_gate_blocked() {
    let workspace = TempWorkspace::new("pending-gate-stage-test");

    let run_dir = workspace.child("run-gate");
    fs::create_dir_all(&run_dir).unwrap();
    fs::write(
        run_dir.join("run-state.json"),
        serde_json::to_string(&bare_run_state(json!({ "pending_gate": "plan" }))).unwrap(),
    )
    .unwrap();
    assert_eq!(pending_gate_stage(&run_dir), Some("plan".to_string()));
    assert!(run_is_gate_blocked(&run_dir));

    let run_dir_empty = workspace.child("run-empty-gate");
    fs::create_dir_all(&run_dir_empty).unwrap();
    fs::write(
        run_dir_empty.join("run-state.json"),
        serde_json::to_string(&bare_run_state(json!({ "pending_gate": "" }))).unwrap(),
    )
    .unwrap();
    assert_eq!(pending_gate_stage(&run_dir_empty), None);
    assert!(!run_is_gate_blocked(&run_dir_empty));

    let run_dir_missing = workspace.child("run-missing-state");
    fs::create_dir_all(&run_dir_missing).unwrap();
    assert_eq!(pending_gate_stage(&run_dir_missing), None);
    assert!(!run_is_gate_blocked(&run_dir_missing));
}

#[test]
fn sleep_unless_stopped_returns_early_on_shutdown() {
    use std::sync::Arc;
    use std::thread;
    use std::time::Instant;

    let shutdown = Arc::new(crate::shutdown::Shutdown::new());
    let shutdown_clone = Arc::clone(&shutdown);

    // Spawn a thread to set the flag after ~50ms
    let setter = thread::spawn(move || {
        thread::sleep(std::time::Duration::from_millis(50));
        shutdown_clone.set();
    });

    let start = Instant::now();
    // Request a 5-second sleep but interrupt it after ~50ms
    let completed = sleep_unless_stopped(&shutdown, std::time::Duration::from_secs(5));
    let elapsed = start.elapsed();

    setter.join().unwrap();

    // Should have returned false (was interrupted)
    assert!(!completed);
    // Should have returned in well under 1 second (we requested early interruption at ~50ms)
    assert!(elapsed < std::time::Duration::from_secs(1));
    // Should be closer to 50-200ms than to 5 seconds
    assert!(elapsed < std::time::Duration::from_secs(1));
}

#[test]
fn negative_cache_ttl_starts_at_initial_ttl() {
    assert_eq!(negative_cache_ttl(1), Duration::from_secs(10));
}

#[test]
fn negative_cache_ttl_doubles_per_miss() {
    assert_eq!(negative_cache_ttl(2), Duration::from_secs(20));
    assert_eq!(negative_cache_ttl(3), Duration::from_secs(40));
}

#[test]
fn negative_cache_ttl_is_capped_at_max_ttl() {
    assert_eq!(negative_cache_ttl(6), NEGATIVE_CACHE_MAX_TTL);
    assert_eq!(negative_cache_ttl(40), NEGATIVE_CACHE_MAX_TTL);
}

#[test]
fn negative_cache_ttl_does_not_overflow_on_max_misses() {
    assert_eq!(negative_cache_ttl(u32::MAX), NEGATIVE_CACHE_MAX_TTL);
}

fn write_bare_run_state(run_dir: &Path, extra: Value) {
    fs::create_dir_all(run_dir).unwrap();
    let state = bare_run_state(extra);
    fs::write(
        run_dir.join("run-state.json"),
        serde_json::to_string(&state).unwrap(),
    )
    .unwrap();
}

#[test]
fn normalize_reports_unpersisted_anchor_from_enveloped_result() {
    let workspace = TempWorkspace::new("anchor-unpersisted");
    let run_dir = workspace.child("run");
    write_bare_run_state(&run_dir, json!({ "workflow_run_id": null }));
    fs::write(
        run_dir.join("workflow-result.json"),
        serde_json::to_string(&json!({
            "runId": "wf-9",
            "result": { "status": "awaiting_gate", "gate": "pr" }
        }))
        .unwrap(),
    )
    .unwrap();
    let normalized = normalize(&run_dir).unwrap();
    assert_eq!(normalized["status"], json!("paused"));
    let gate = &normalized["currentGate"];
    assert_eq!(gate["stage"], json!("pr"));
    assert_eq!(gate["anchorPersisted"], json!(false));
    assert!(gate["resumeCommand"]
        .as_str()
        .unwrap()
        .contains("aidp run record-resume --run run-anchor --from-result"));
}

#[test]
fn normalize_marks_persisted_pending_gate_as_anchored() {
    let workspace = TempWorkspace::new("anchor-persisted");
    let run_dir = workspace.child("run");
    write_bare_run_state(&run_dir, json!({ "pending_gate": "pr" }));
    let normalized = normalize(&run_dir).unwrap();
    assert_eq!(normalized["status"], json!("paused"));
    assert_eq!(normalized["currentGate"]["anchorPersisted"], json!(true));
}

#[test]
fn it_normalize_ignores_malformed_result_file() {
    let workspace = TempWorkspace::new("anchor-malformed");
    let run_dir = workspace.child("run");
    write_bare_run_state(&run_dir, json!({}));
    fs::write(run_dir.join("workflow-result.json"), "{not json").unwrap();
    let normalized = normalize(&run_dir).unwrap();
    assert_eq!(normalized["status"], json!("pending"));
    assert!(normalized["currentGate"].is_null());
}

const PAYLOAD_SNAPSHOT_FILE: &str = "tests/fixtures/run_payload_snapshot.json";
const SNAPSHOT_RUN_DIR_PLACEHOLDER: &str = "<run_dir>";

fn snapshot_run_state() -> Value {
    json!({
        "run_id": "20260929-101500-snap",
        "pipeline_path": "templates/dev-aidp.yaml",
        "pipeline_id": "dev-aidp",
        "workspace_root": "C:/code/repo-snap",
        "run_dir": "unused",
        "started_at": "2026-09-29T10:15:00Z",
        "repo": "repo-snap",
        "feature": "feature-snap",
        "stages": [
            {
                "name": "plan",
                "status": "success",
                "started_at": "2026-09-29T10:15:01Z",
                "finished_at": "2026-09-29T10:16:01Z",
                "duration_ms": 60000,
                "tokens_used": 1200,
                "stage_dir": "plan"
            },
            {
                "name": "implement",
                "status": "in_progress",
                "started_at": "2026-09-29T10:16:02Z",
                "stage_dir": "implement"
            }
        ],
    })
}

#[test]
fn it_normalize_payload_matches_snapshot() {
    let workspace = TempWorkspace::new("payload-snapshot");
    let run_dir = workspace.child("run-snap");
    fs::create_dir_all(&run_dir).unwrap();
    fs::write(
        run_dir.join("run-state.json"),
        serde_json::to_string(&snapshot_run_state()).unwrap(),
    )
    .unwrap();

    let mut payload = normalize(&run_dir).unwrap();
    assert_eq!(payload.as_object().unwrap().len(), 13);
    assert_eq!(payload["status"], json!("running"));
    assert_eq!(payload["stageCount"], json!(2));
    assert_eq!(payload["doneCount"], json!(1));
    payload["runDir"] = json!(SNAPSHOT_RUN_DIR_PLACEHOLDER);

    let actual = serde_json::to_string(&payload).unwrap();
    let expected =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join(PAYLOAD_SNAPSHOT_FILE))
            .unwrap();
    assert_eq!(actual, expected);
}

#[test]
fn normalize_accepts_old_format_run_state() {
    let workspace = TempWorkspace::new("old-format");
    let run_dir = workspace.child("run-old");
    fs::create_dir_all(&run_dir).unwrap();
    let old_state = json!({
        "run_id": "run-old",
        "pipeline_id": "dev-aidp",
        "pipeline_path": "templates/dev-aidp.yaml",
        "workspace_root": "C:/code/repo-old",
        "run_dir": "run-old",
        "started_at": "2025-01-01T00:00:00Z",
        "repo": "repo-old",
        "feature": "feature-old",
        "stages": [],
    });
    fs::write(
        run_dir.join("run-state.json"),
        serde_json::to_string(&old_state).unwrap(),
    )
    .unwrap();

    let normalized = normalize(&run_dir).unwrap();
    assert_eq!(normalized["runId"], json!("run-old"));
    assert_eq!(normalized["pipelineId"], json!("dev-aidp"));
    assert_eq!(normalized["repo"], json!("repo-old"));
    assert_eq!(normalized["feature"], json!("feature-old"));
    assert_eq!(normalized["status"], json!("pending"));
    assert_eq!(normalized["workflowRunId"], Value::Null);
}

#[test]
fn read_run_state_skips_file_missing_a_required_field() {
    let workspace = TempWorkspace::new("missing-field");
    let run_dir = workspace.child("run-missing");
    fs::create_dir_all(&run_dir).unwrap();
    let incomplete = json!({
        "run_id": "run-missing",
        "pipeline_id": "dev-aidp",
        "workspace_root": "C:/code/repo-missing",
        "run_dir": "run-missing",
        "started_at": "2025-01-01T00:00:00Z",
        "stages": [],
    });
    fs::write(
        run_dir.join("run-state.json"),
        serde_json::to_string(&incomplete).unwrap(),
    )
    .unwrap();

    assert!(read_run_state(&run_dir).is_none());
    assert!(normalize(&run_dir).is_none());
}
