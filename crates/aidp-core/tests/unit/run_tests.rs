use super::*;

#[test]
fn stage_segment_keeps_real_stage_names() {
    for n in [
        "qa__explore",
        "knowledge_feature_update",
        "plan",
        "implement",
    ] {
        assert_eq!(stage_segment(n), n);
    }
}

#[test]
fn inner_double_dot_in_one_component_is_schema_valid_and_safe() {
    for n in ["a..b", "step..2", "x.."] {
        assert!(is_safe_stage_name(n), "{n} should be schema-safe");
        assert_eq!(stage_segment(n), n);
    }
}

#[test]
fn distinct_invalid_names_get_distinct_segments() {
    let a = stage_segment("..");
    let b = stage_segment("///");
    assert_ne!(a, b, "distinct invalid names must not share a directory");
    assert!(a.starts_with(INVALID_STAGE_SEGMENT));
    assert!(b.starts_with(INVALID_STAGE_SEGMENT));
    assert_eq!(a, stage_segment(".."));
}

#[test]
fn stage_dir_traversal_stays_under_run_dir() {
    let run_dir = PathBuf::from("/vault/Pipelines/r/F/run1");
    for bad in ["../../escape", "..", "a/b", "a\\b", "", "_x", "/abs"] {
        let seg = stage_segment(bad);
        assert!(
            !seg.contains(['/', '\\']) && !seg.contains(".."),
            "{bad} -> {seg}"
        );
        assert!(!seg.starts_with('_') && !seg.is_empty(), "{bad} -> {seg}");
        let p = run_dir.join(&seg);
        assert_eq!(p.parent(), Some(run_dir.as_path()), "{bad}");
    }
}

#[test]
fn run_state_validates_against_schema() {
    let mut outputs = BTreeMap::new();
    outputs.insert("context".to_string(), PathBuf::from("/abs/context.md"));
    let mut loops = BTreeMap::new();
    loops.insert("correction_loop".to_string(), LoopState { iter: 1 });
    let st = RunState {
        run_id: "20260619-101010-abcd".into(),
        pipeline_path: PathBuf::from("templates/dev.yaml"),
        pipeline_id: "dev".into(),
        workspace_root: PathBuf::from("/ws"),
        run_dir: PathBuf::from("/ws/.aidp/pipelines/20260619-101010-abcd"),
        started_at: "2026-06-19T10:10:10Z".into(),
        plan: vec![vec!["discover".into()]],
        stages: vec![StageState {
            name: "discover".into(),
            status: StageStatus::Success,
            started_at: Some("2026-06-19T10:10:11Z".into()),
            finished_at: Some("2026-06-19T10:10:20Z".into()),
            duration_ms: Some(9000),
            tokens_used: Some(1234),
            stage_dir: PathBuf::from("/ws/.aidp/pipelines/20260619-101010-abcd/discover"),
            outputs,
            summary: Some("Explored the alerts service and mapped its handlers.".into()),
            follow_ups: vec!["Confirm the retention policy with the data team.".into()],
            usage: Some(StageUsage {
                input: 3,
                output: 728,
                cache_creation: 60_317,
                cache_read: 19_236,
                model: Some("claude-sonnet-5".into()),
                agent_type: Some("executor".into()),
                model_pinned: Some("claude-sonnet-5".into()),
                bucket: ComplexityBucket::Medium,
                cost_usd: Some(0.42),
                pricing_version: Some("2026-09-01".into()),
                source: UsageSource::Jsonl,
                tool_calls: Some(7),
            }),
            verification: vec!["/abs/context.md: too-short (12 bytes)".into()],
            skills_not_loaded: Vec::new(),
        }],
        loops,
        workflow_run_id: Some("wf_abc123".into()),
        gates: vec![GateDecision {
            after_stage: "plan".into(),
            asked_at: Some("2026-06-19T10:11:00Z".into()),
            decision: GateDecisionKind::Approve,
            notes: None,
        }],
        pending_gate: Some("pr".into()),
        last_workflow_status: Some("awaiting_gate".into()),
        vault_root: Some(PathBuf::from("C:/CODE/AIDP")),
        knowledge_root: Some(PathBuf::from("C:/CODE/AIDP/Knowledge/cheat-alerts-service")),
        repo: Some("cheat-alerts-service".into()),
        feature: Some("PIXEL-1234".into()),
        plan_schedules: {
            let mut m = BTreeMap::new();
            m.insert(
                "implement".to_string(),
                crate::plan::ScheduleState {
                    stage: "implement".into(),
                    plan_path: "/abs/plan/plan.json".into(),
                    compiled_at: "2026-06-19T10:12:00Z".into(),
                    max_parallel: 4,
                    width: 2,
                    waves: vec![vec!["T1".into(), "T2".into()], vec!["T3".into()]],
                    serializations: vec![crate::plan::Serialization {
                        before: "T1".into(),
                        after: "T2".into(),
                        files: vec!["src/Shared.cs".into()],
                    }],
                    warnings: vec!["T3 declares no owned file".into()],
                    tasks: vec![crate::plan::TaskState {
                        id: "T1".into(),
                        title: "add the handler".into(),
                        kind: "code".into(),
                        wave: 0,
                        deps: vec![],
                        mutex_after: vec!["T0".into()],
                        files: vec!["src/A.cs".into()],
                        status: crate::plan::TaskStatus::Success,
                        tokens_used: Some(2100),
                    }],
                },
            );
            m
        },
        cost: Some(RunCost {
            total_usd: Some(0.42),
            tokens_billable: 80_284,
            pricing_version: Some("2026-09-01".into()),
            captured_at: "2026-06-19T10:13:00Z".into(),
            by_model: {
                let mut m = BTreeMap::new();
                m.insert(
                    "claude-sonnet-5".to_string(),
                    StageUsage {
                        input: 3,
                        output: 728,
                        cache_creation: 60_317,
                        cache_read: 19_236,
                        model: Some("claude-sonnet-5".into()),
                        agent_type: Some("executor".into()),
                        model_pinned: Some("claude-sonnet-5".into()),
                        bucket: ComplexityBucket::Medium,
                        cost_usd: Some(0.42),
                        pricing_version: Some("2026-09-01".into()),
                        source: UsageSource::Jsonl,
                        tool_calls: Some(7),
                    },
                );
                m
            },
        }),
        base_sha: Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into()),
        head_sha_end: Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into()),
        branch: Some("feat/PIXEL-1234".into()),
        commits: vec![
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
        ],
        scope_violations: vec![crate::scope::Violation {
            path: "src/OutOfScope.cs".into(),
            reason: "not in tests: src/OutOfScope.cs".into(),
        }],
        guard_findings: vec![serde_json::json!({
            "source": "fanin:review",
            "findings": ["instruction-override:critical"],
            "bytes": 812
        })],
        ..Default::default()
    };
    let value = serde_json::to_value(&st).unwrap();
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("../../../../schema/run-state.schema.json")).unwrap();
    let compiled = jsonschema::JSONSchema::options()
        .with_draft(jsonschema::Draft::Draft7)
        .compile(&schema)
        .expect("run-state schema compiles");
    let errs: Vec<String> = match compiled.validate(&value) {
        Ok(()) => Vec::new(),
        Err(errors) => errors
            .map(|e| format!("{}: {}", e.instance_path, e))
            .collect(),
    };
    assert!(
        errs.is_empty(),
        "serialized RunState does not validate against run-state.schema.json:\n  {}",
        errs.join("\n  ")
    );
}

#[test]
fn run_state_without_the_six_new_fields_still_loads() {
    let json = r#"{
            "run_id": "20260101-000000-0000",
            "pipeline_path": "templates/dev.yaml",
            "pipeline_id": "dev",
            "workspace_root": "/ws",
            "run_dir": "/ws/.aidp/pipelines/20260101-000000-0000",
            "started_at": "2026-01-01T00:00:00Z",
            "stages": [
                {
                    "name": "discover",
                    "status": "success",
                    "stage_dir": "/ws/.aidp/pipelines/20260101-000000-0000/discover"
                }
            ]
        }"#;
    let st: RunState =
        serde_json::from_str(json).expect("a pre-C7 run-state.json must still deserialize");
    assert!(st.base_sha.is_none());
    assert!(st.head_sha_end.is_none());
    assert!(st.branch.is_none());
    assert!(st.commits.is_empty());
    assert!(st.scope_violations.is_empty());
    assert!(st.stages[0].verification.is_empty());
}

#[test]
fn feature_key_derives_a_stable_feature_segment() {
    let fk = |v: serde_json::Value| {
        let m: BTreeMap<String, serde_json::Value> = match v {
            serde_json::Value::Object(o) => o.into_iter().collect(),
            _ => BTreeMap::new(),
        };
        feature_key(&m)
    };
    assert_eq!(
        fk(serde_json::json!({"jira_keys": ["PIXEL-2", "PIXEL-1"]})).as_deref(),
        Some("PIXEL-1+PIXEL-2")
    );
    assert_eq!(
        fk(serde_json::json!({"jira_keys": ["A-1", "A-1"]})).as_deref(),
        Some("A-1")
    );
    assert_eq!(
        fk(serde_json::json!({"feature": "EPIC-9", "jira_keys": ["A-1", "A-2"]})).as_deref(),
        Some("EPIC-9")
    );
    assert_eq!(
        fk(serde_json::json!({"jira_keys": ["K-1","K-2","K-3","K-4","K-5"]})).as_deref(),
        Some("K-1+4more")
    );
    assert_eq!(
        fk(serde_json::json!({"jira_key": "PIXEL-42"})).as_deref(),
        Some("PIXEL-42")
    );
    assert_eq!(
        fk(serde_json::json!({"target": "T/1 x"})).as_deref(),
        Some("T-1-x")
    );
    assert_eq!(fk(serde_json::json!({})), None);
    assert_eq!(
        fk(serde_json::json!({"jira_keys": [], "jira_key": "AB-3"})).as_deref(),
        Some("AB-3")
    );
}

#[test]
fn parse_auto_approve_gates_splits_trims_and_drops_empties() {
    assert_eq!(
        parse_auto_approve_gates(" pr , , plan "),
        vec!["pr".to_string(), "plan".to_string()]
    );
    assert_eq!(parse_auto_approve_gates("*"), vec!["*".to_string()]);
    assert!(parse_auto_approve_gates("").is_empty());
}

#[test]
fn run_state_with_vault_leaks_round_trips_serde() {
    let mut outputs = BTreeMap::new();
    outputs.insert("changed".to_string(), PathBuf::from("/s/changed.json"));
    let st = RunState {
        run_id: "20260919-000000-abcd".into(),
        pipeline_path: PathBuf::from("templates/dev.yaml"),
        pipeline_id: "dev".into(),
        workspace_root: PathBuf::from("/ws"),
        run_dir: PathBuf::from("/ws/.aidp/pipelines/20260919-000000-abcd"),
        started_at: "2026-09-19T00:00:00Z".into(),
        vault_leaks: vec![
            crate::vault_leak::VaultLeak {
                path: "Pipelines/ai-dev-pipeline-v7/loc-46/20260919-000000-abcd/impl/T3.md".into(),
                kind: crate::vault_leak::LeakKind::RunArtifact,
                reason: "run artefacts belong under <vault>/Pipelines/, never in the repo tree"
                    .into(),
            },
            crate::vault_leak::VaultLeak {
                path: "Knowledge/ai-dev-pipeline-v7/architecture.md".into(),
                kind: crate::vault_leak::LeakKind::KnowledgeNote,
                reason: "knowledge notes belong under <vault>/Knowledge/, never in the repo tree"
                    .into(),
            },
        ],
        stages: vec![StageState {
            name: "code".into(),
            status: StageStatus::Success,
            stage_dir: PathBuf::from("/ws/code"),
            outputs,
            ..Default::default()
        }],
        ..Default::default()
    };
    let json = serde_json::to_string(&st).unwrap();
    let restored: RunState = serde_json::from_str(&json).unwrap();
    assert_eq!(restored.vault_leaks.len(), 2);
    assert_eq!(
        restored.vault_leaks[0].kind,
        crate::vault_leak::LeakKind::RunArtifact
    );
    assert_eq!(
        restored.vault_leaks[1].kind,
        crate::vault_leak::LeakKind::KnowledgeNote
    );
}

#[test]
fn run_state_without_vault_leaks_field_deserializes_with_empty_vec() {
    let json = r#"{
            "run_id": "20260919-000000-0000",
            "pipeline_path": "templates/dev.yaml",
            "pipeline_id": "dev",
            "workspace_root": "/ws",
            "run_dir": "/ws/.aidp/pipelines/20260919-000000-0000",
            "started_at": "2026-09-19T00:00:00Z",
            "stages": [
                {
                    "name": "code",
                    "status": "success",
                    "stage_dir": "/ws/code"
                }
            ]
        }"#;
    let st: RunState =
        serde_json::from_str(json).expect("pre-LOC-46 run-state.json must still deserialize");
    assert!(st.vault_leaks.is_empty());
}

#[test]
fn render_summary_includes_vault_leak_line_when_present() {
    let mut st = RunState {
        run_id: "20260919-000000-abcd".into(),
        pipeline_path: PathBuf::from("templates/dev.yaml"),
        pipeline_id: "dev".into(),
        workspace_root: PathBuf::from("/ws"),
        run_dir: PathBuf::from("/ws/.aidp/pipelines/20260919-000000-abcd"),
        started_at: "2026-09-19T00:00:00Z".into(),
        stages: vec![StageState {
            name: "code".into(),
            status: StageStatus::Success,
            stage_dir: PathBuf::from("/ws/code"),
            ..Default::default()
        }],
        ..Default::default()
    };

    let summary = render_summary(&st);
    assert!(
        !summary.contains("vault leak"),
        "summary must not mention vault leaks when none are present:\n{summary}"
    );

    st.vault_leaks = vec![crate::vault_leak::VaultLeak {
        path: "Pipelines/ai-dev-pipeline-v7/loc-46/20260919-000000-abcd/code/T1.md".into(),
        kind: crate::vault_leak::LeakKind::RunArtifact,
        reason: "run artefacts belong under <vault>/Pipelines/, never in the repo tree".into(),
    }];
    let summary_with_leak = render_summary(&st);
    assert!(
        summary_with_leak.contains("⛔ vault leak"),
        "summary must contain the vault-leak line when leaks are present:\n{summary_with_leak}"
    );
    assert!(
        summary_with_leak.contains("aidp scope verify"),
        "summary must mention scope verify for details:\n{summary_with_leak}"
    );
}

#[test]
fn run_state_json_without_skills_not_loaded_deserializes_to_an_empty_list() {
    let json = r#"{
            "run_id": "20260101-000000-0000",
            "pipeline_path": "templates/dev.yaml",
            "pipeline_id": "dev",
            "workspace_root": "/ws",
            "run_dir": "/ws/run",
            "started_at": "2026-01-01T00:00:00Z",
            "stages": [
                { "name": "discover", "status": "success", "stage_dir": "/ws/run/discover" }
            ]
        }"#;
    let st: RunState = serde_json::from_str(json).unwrap();
    assert!(st.stages[0].skills_not_loaded.is_empty());
}

const BARE_GATE_RESULT: &str = r#"{"status":"awaiting_gate","gate":"plan","runId":"wf_7"}"#;

#[test]
fn bare_result_yields_run_id_and_gate() {
    let anchor = parse_resume_anchor(BARE_GATE_RESULT).unwrap();
    assert_eq!(anchor.workflow_run_id.as_deref(), Some("wf_7"));
    assert_eq!(anchor.pending_gate.as_deref(), Some("plan"));
    assert_eq!(anchor.workflow_status.as_deref(), Some("awaiting_gate"));
}

#[test]
fn enveloped_result_yields_the_same_anchor() {
    let raw = r#"{"result":{"status":"awaiting_gate","gate":"plan","runId":"wf_7"}}"#;
    assert_eq!(
        parse_resume_anchor(raw).unwrap(),
        parse_resume_anchor(BARE_GATE_RESULT).unwrap()
    );
}

#[test]
fn outer_run_id_is_used_when_inner_has_none() {
    let raw = r#"{"runId":"wf_outer","result":{"status":"awaiting_gate","gate":"pr"}}"#;
    let anchor = parse_resume_anchor(raw).unwrap();
    assert_eq!(anchor.workflow_run_id.as_deref(), Some("wf_outer"));
    assert_eq!(anchor.pending_gate.as_deref(), Some("pr"));
}

#[test]
fn inner_run_id_wins_over_outer() {
    let raw = r#"{"runId":"wf_outer","result":{"status":"done","runId":"wf_inner"}}"#;
    let anchor = parse_resume_anchor(raw).unwrap();
    assert_eq!(anchor.workflow_run_id.as_deref(), Some("wf_inner"));
}

#[test]
fn non_gate_statuses_keep_run_id_without_pending_gate() {
    for status in ["done", "paused"] {
        let raw = format!(r#"{{"status":"{status}","gate":"plan","runId":"wf_2"}}"#);
        let anchor = parse_resume_anchor(&raw).unwrap();
        assert_eq!(anchor.workflow_run_id.as_deref(), Some("wf_2"));
        assert_eq!(anchor.pending_gate, None);
        assert_eq!(anchor.workflow_status.as_deref(), Some(status));
    }
}

#[test]
fn empty_run_id_and_empty_gate_count_as_absent() {
    let raw = r#"{"status":"awaiting_gate","gate":"","runId":""}"#;
    let anchor = parse_resume_anchor(raw).unwrap();
    assert_eq!(anchor.workflow_run_id, None);
    assert_eq!(anchor.pending_gate, None);
}

const REVISE_GATE_JSON: &str = r#"{"after_stage":"plan","decision":"revise"}"#;

#[test]
fn a_schema_valid_revise_decision_still_loads() {
    let gate: GateDecision =
        serde_json::from_str(REVISE_GATE_JSON).expect("revise is schema-valid");
    assert_eq!(gate.decision, GateDecisionKind::Revise);
    assert_eq!(serde_json::to_value(gate.decision).unwrap(), "revise");
}

const RECONCILE_SPAN_NAME: &str = "reconcile";
const RUN_ID_FIELD: &str = "run_id";
const SPAN_RUN_ID: &str = "r-span";

/// One span captured by `SpanRecorder`: its name and the `run_id` field value.
#[derive(Default, Clone)]
struct CapturedSpan {
    name: String,
    run_id: Option<String>,
}

/// Visitor copying the `run_id` field of a span into a `CapturedSpan`.
struct RunIdVisitor<'a>(&'a mut CapturedSpan);

impl tracing::field::Visit for RunIdVisitor<'_> {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == RUN_ID_FIELD {
            self.0.run_id = Some(format!("{value:?}"));
        }
    }
}

/// Layer recording every span opened while it is the active subscriber.
#[derive(Clone, Default)]
struct SpanRecorder(std::sync::Arc<std::sync::Mutex<Vec<CapturedSpan>>>);

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for SpanRecorder {
    fn on_new_span(
        &self,
        attrs: &tracing::span::Attributes<'_>,
        _id: &tracing::span::Id,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let mut captured = CapturedSpan {
            name: attrs.metadata().name().to_string(),
            run_id: None,
        };
        attrs.record(&mut RunIdVisitor(&mut captured));
        self.0.lock().unwrap().push(captured);
    }
}

/// Reconcile opens a span named after the function carrying the run id.
#[test]
fn reconcile_opens_span_with_run_id_field() {
    use tracing_subscriber::layer::SubscriberExt;
    let recorder = SpanRecorder::default();
    let subscriber = tracing_subscriber::registry().with(recorder.clone());
    let run_dir = std::env::temp_dir().join(format!("aidp-core-span-{}", std::process::id()));
    std::fs::create_dir_all(&run_dir).unwrap();
    let mut state = RunState {
        run_id: SPAN_RUN_ID.into(),
        workspace_root: run_dir.clone(),
        run_dir,
        ..Default::default()
    };
    let result = serde_json::json!({ "status": "paused", "results": {} });
    tracing::subscriber::with_default(subscriber, || {
        let _ = reconcile(&mut state, &result);
    });
    let spans = recorder.0.lock().unwrap();
    let span = spans
        .iter()
        .find(|s| s.name == RECONCILE_SPAN_NAME)
        .expect("reconcile span opened");
    assert_eq!(span.run_id.as_deref(), Some(SPAN_RUN_ID));
}
