use aidp_core::run::*;
use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::common;
use common::*;

#[test]
fn reconcile_without_jsonl_records_source_budget() {
    let mk = |name: &str| StageState {
        name: name.into(),
        stage_dir: PathBuf::from(format!("/ws/{name}")),
        ..Default::default()
    };
    let run_dir = scratch_dir();
    let mut st = RunState {
        run_id: "r-no-jsonl".into(),
        pipeline_path: PathBuf::from("/nonexistent.yaml"),
        pipeline_id: "dev".into(),
        workspace_root: run_dir.clone(),
        run_dir: run_dir.clone(),
        started_at: "t".into(),
        stages: vec![mk("code")],
        ..Default::default()
    };
    let result = serde_json::json!({
        "status": "done",
        "results": {
            "code": {
                "status": "completed", "summary": "did it",
                "metrics": { "tokens_used": 4200 }
            }
        }
    });
    let outcome = reconcile(&mut st, &result);
    assert!(outcome.is_ok(), "reconcile must succeed: {:?}", outcome);
    let code = st.stages.iter().find(|s| s.name == "code").unwrap();
    let usage = code.usage.as_ref().expect("usage attached");
    assert_eq!(usage.source, UsageSource::Budget);
    assert_eq!(usage.cost_usd, None);
}

#[test]
fn reconcile_gated_run_keeps_unreached_stages_pending() {
    let mk = |name: &str| StageState {
        name: name.into(),
        stage_dir: PathBuf::from(format!("/ws/{name}")),
        ..Default::default()
    };
    let run_dir = scratch_dir();
    let mut st = RunState {
        run_id: "r".into(),
        pipeline_path: PathBuf::from("/nonexistent.yaml"),
        pipeline_id: "dev".into(),
        workspace_root: run_dir.clone(),
        run_dir: run_dir.clone(),
        started_at: "t".into(),
        stages: vec![mk("code"), mk("pr")],
        pending_gate: Some("pr".into()),
        ..Default::default()
    };
    let result = serde_json::json!({
        "status": "awaiting_gate",
        "gate": "pr",
        "results": {
            "code": { "status": "completed", "metrics": { "tokens_used": 42 } }
        }
    });
    reconcile(&mut st, &result).unwrap();
    let code = st.stages.iter().find(|s| s.name == "code").unwrap();
    assert_eq!(code.status, StageStatus::Success);
    assert_eq!(code.tokens_used, Some(42));
    assert_eq!(
        st.stages.iter().find(|s| s.name == "pr").unwrap().status,
        StageStatus::Pending,
        "unreached stage must stay Pending, never Skipped, on a non-finished run"
    );
    assert_eq!(st.last_workflow_status.as_deref(), Some("awaiting_gate"));
}

#[test]
fn reconcile_clears_a_stale_verification_signal_once_the_output_is_fixed() {
    let dir = scratch_dir();
    std::fs::create_dir_all(&dir).unwrap();
    let out_path = dir.join("summary.md");
    std::fs::write(&out_path, "").unwrap(); // empty → triggers `empty-file`

    let mut outputs = BTreeMap::new();
    outputs.insert("summary".to_string(), out_path.clone());
    let mut st = RunState {
        run_id: "r".into(),
        pipeline_path: PathBuf::from("/nonexistent.yaml"),
        pipeline_id: "dev".into(),
        workspace_root: dir.clone(),
        run_dir: dir.clone(),
        started_at: "t".into(),
        stages: vec![StageState {
            name: "plan".into(),
            stage_dir: dir.clone(),
            outputs,
            ..Default::default()
        }],
        ..Default::default()
    };
    let result = serde_json::json!({ "status": "done", "results": {} });

    reconcile(&mut st, &result).unwrap();
    let stage = &st.stages[0];
    assert!(
        !stage.verification.is_empty(),
        "an empty output file must produce a verification signal"
    );
    assert!(
        stage.verification.iter().any(|s| s.contains("empty-file")),
        "got: {:?}",
        stage.verification
    );

    std::fs::write(
        &out_path,
        "# Summary\n\nEverything is fine now, plenty of real content here.\n",
    )
    .unwrap();
    reconcile(&mut st, &result).unwrap();
    let stage = &st.stages[0];
    assert_eq!(
        stage.verification,
        Vec::<String>::new(),
        "a healthy output must clear the stale signal, not just leave it stuck at the old value"
    );
}

#[test]
fn reconcile_folds_plan_task_results() {
    let run_dir = scratch_dir();
    let mk_task = |id: &str| aidp_core::plan::TaskState {
        id: id.into(),
        title: format!("task {id}"),
        kind: "code".into(),
        wave: 0,
        deps: vec![],
        mutex_after: vec![],
        files: vec![format!("src/{id}.cs")],
        status: aidp_core::plan::TaskStatus::Pending,
        tokens_used: None,
    };
    let mut schedules = BTreeMap::new();
    schedules.insert(
        "implement".to_string(),
        aidp_core::plan::ScheduleState {
            stage: "implement".into(),
            plan_path: "/abs/plan.json".into(),
            compiled_at: "t".into(),
            max_parallel: 4,
            width: 3,
            waves: vec![vec!["T1".into(), "T2".into(), "T3".into()]],
            serializations: vec![],
            warnings: vec![],
            tasks: vec![mk_task("T1"), mk_task("T2"), mk_task("T3")],
        },
    );
    let mut st = RunState {
        run_id: "r".into(),
        pipeline_path: PathBuf::from("/nonexistent.yaml"),
        pipeline_id: "dev".into(),
        workspace_root: run_dir.clone(),
        run_dir: run_dir.clone(),
        started_at: "t".into(),
        stages: vec![StageState {
            name: "implement".into(),
            stage_dir: run_dir.join("implement"),
            ..Default::default()
        }],
        plan_schedules: schedules,
        ..Default::default()
    };
    let result = serde_json::json!({
        "status": "done",
        "results": {
            "implement": { "status": "completed", "summary": "merged" },
            "implement#T1": { "status": "completed", "summary": "ok", "metrics": { "tokens_used": 900 } },
            "implement#T2": { "status": "failed", "summary": "nope" }
        }
    });
    reconcile(&mut st, &result).unwrap();
    let tasks = &st.plan_schedules["implement"].tasks;
    let t = |id: &str| tasks.iter().find(|t| t.id == id).unwrap();
    assert_eq!(t("T1").status, aidp_core::plan::TaskStatus::Success);
    assert_eq!(t("T1").tokens_used, Some(900));
    assert_eq!(t("T2").status, aidp_core::plan::TaskStatus::Failed);
    assert_eq!(t("T3").status, aidp_core::plan::TaskStatus::Skipped);
    assert_eq!(st.stages[0].status, StageStatus::Success);
    let sum = render_summary(&st);
    assert!(
        sum.contains("plan `implement`: 1/3 task(s) succeeded"),
        "got: {sum}"
    );
}

/// A run whose `implement` stage is plan-driven over `ids`, ready to reconcile.
/// The task-accounting tests below care only about its schedule and status.
fn planned_run(tag: &str, ids: &[&str]) -> RunState {
    let run_dir = scratch_dir();
    let mut schedules = BTreeMap::new();
    schedules.insert(
        "implement".to_string(),
        aidp_core::plan::ScheduleState {
            stage: "implement".into(),
            plan_path: "/abs/plan.json".into(),
            compiled_at: "t".into(),
            max_parallel: 3,
            width: ids.len(),
            waves: vec![ids.iter().map(|s| s.to_string()).collect()],
            serializations: vec![],
            warnings: vec![],
            tasks: ids
                .iter()
                .map(|id| aidp_core::plan::TaskState {
                    id: (*id).into(),
                    title: format!("task {id}"),
                    kind: "code".into(),
                    wave: 0,
                    deps: vec![],
                    mutex_after: vec![],
                    files: vec![format!("src/{id}.cs")],
                    status: aidp_core::plan::TaskStatus::Pending,
                    tokens_used: None,
                })
                .collect(),
        },
    );
    RunState {
        run_id: tag.into(),
        pipeline_path: PathBuf::from("/nonexistent.yaml"),
        pipeline_id: "dev".into(),
        workspace_root: run_dir.clone(),
        run_dir: run_dir.clone(),
        started_at: "t".into(),
        stages: vec![StageState {
            name: "implement".into(),
            stage_dir: run_dir.join("implement"),
            ..Default::default()
        }],
        plan_schedules: schedules,
        ..Default::default()
    }
}

#[test]
fn plan_task_accounting_holds_for_one_task_and_for_many() {
    let mut one = planned_run("acct1", &["T1"]);
    let env = serde_json::json!({
        "status": "completed",
        "summary": "documented the limitation",
        "metrics": { "tokens_used": 24_000 }
    });
    let anomalies = reconcile(
        &mut one,
        &serde_json::json!({
            "status": "done",
            "results": { "implement": env.clone(), "implement#T1": env }
        }),
    )
    .unwrap();
    assert_eq!(
        one.plan_schedules["implement"].tasks[0].status,
        aidp_core::plan::TaskStatus::Success
    );
    assert_eq!(
        one.plan_schedules["implement"].tasks[0].tokens_used,
        Some(24_000)
    );
    let sum = render_summary(&one);
    assert!(
        sum.contains("plan `implement`: 1/1 task(s) succeeded"),
        "got: {sum}"
    );
    assert!(
        !sum.contains("⚠ inconsistent"),
        "a truthful count carries no warning: {sum}"
    );
    assert!(anomalies.is_empty(), "nothing to report: {anomalies:?}");

    let mut many = planned_run("acctN", &["T1", "T2", "T3"]);
    let anomalies = reconcile(
        &mut many,
        &serde_json::json!({
            "status": "done",
            "results": {
                "implement": { "status": "completed", "summary": "merged" },
                "implement#T1": { "status": "completed", "summary": "a" },
                "implement#T2": { "status": "completed", "summary": "b" },
                "implement#T3": { "status": "needs_review", "summary": "c" }
            }
        }),
    )
    .unwrap();
    let sum = render_summary(&many);
    assert!(
        sum.contains("plan `implement`: 3/3 task(s) succeeded"),
        "got: {sum}"
    );
    assert!(anomalies.is_empty(), "nothing to report: {anomalies:?}");
}

#[test]
fn a_zero_task_count_under_a_successful_stage_is_reported() {
    let mut st = planned_run("acct0", &["T1"]);
    let anomalies = reconcile(
        &mut st,
        &serde_json::json!({
            "status": "done",
            "results": { "implement": { "status": "completed", "summary": "done" } }
        }),
    )
    .unwrap();
    assert_eq!(st.stages[0].status, StageStatus::Success);
    assert_eq!(
        anomalies.len(),
        1,
        "the contradiction must be handed back: {anomalies:?}"
    );
    let a = &anomalies[0];
    assert!(
        a.contains("implement") && a.contains("0/1"),
        "must name stage and count: {a}"
    );
    assert!(a.contains("reporting failure"), "must say what it is: {a}");
    let sum = render_summary(&st);
    assert!(
        sum.contains("plan `implement`: 0/1 task(s) succeeded"),
        "got: {sum}"
    );
    assert!(
        sum.contains("⚠ inconsistent"),
        "the count must not stand alone: {sum}"
    );
}

#[test]
fn a_zero_task_count_is_not_reported_when_the_stage_itself_failed() {
    let mut failed = planned_run("acctko", &["T1", "T2"]);
    let anomalies = reconcile(
        &mut failed,
        &serde_json::json!({
            "status": "done",
            "results": {
                "implement": { "status": "failed", "summary": "broke" },
                "implement#T1": { "status": "failed", "summary": "broke" },
                "implement#T2": { "status": "failed", "summary": "broke" }
            }
        }),
    )
    .unwrap();
    assert_eq!(failed.stages[0].status, StageStatus::Cancel);
    assert!(
        anomalies.is_empty(),
        "a failed stage explains its own 0/N: {anomalies:?}"
    );
    assert!(!render_summary(&failed).contains("⚠ inconsistent"));

    let mut pending = planned_run("acctpend", &["T1"]);
    let anomalies = reconcile(
        &mut pending,
        &serde_json::json!({ "status": "paused", "at": "plan", "results": {} }),
    )
    .unwrap();
    assert_eq!(pending.stages[0].status, StageStatus::Pending);
    assert!(
        anomalies.is_empty(),
        "an unreached stage is not an inconsistency: {anomalies:?}"
    );
}

#[test]
fn carry_over_task_outcomes_restores_what_reconcile_recorded() {
    let mut done = planned_run("acctcarry", &["T1", "T2"]);
    reconcile(
        &mut done,
        &serde_json::json!({
            "status": "done",
            "results": {
                "implement": { "status": "completed", "summary": "merged" },
                "implement#T1": { "status": "completed", "metrics": { "tokens_used": 700 } },
                "implement#T2": { "status": "failed", "summary": "nope" }
            }
        }),
    )
    .unwrap();
    let mut fresh = planned_run("acctcarry2", &["T1", "T2"]).plan_schedules["implement"].clone();
    assert!(
        fresh
            .tasks
            .iter()
            .all(|t| t.status == aidp_core::plan::TaskStatus::Pending),
        "a fresh compile knows nothing"
    );
    carry_over_task_outcomes(&done.plan_schedules["implement"], &mut fresh);
    assert_eq!(fresh.tasks[0].status, aidp_core::plan::TaskStatus::Success);
    assert_eq!(fresh.tasks[0].tokens_used, Some(700));
    assert_eq!(fresh.tasks[1].status, aidp_core::plan::TaskStatus::Failed);
}

#[test]
fn record_plan_schedules_preserves_known_outcomes() {
    let run_dir = scratch_dir();
    let mk = |id: &str, status: &str, tok: Option<u64>| aidp_core::plan::TaskState {
        id: id.into(),
        title: id.into(),
        kind: "code".into(),
        wave: 0,
        deps: vec![],
        mutex_after: vec![],
        files: vec![],
        status: aidp_core::plan::TaskStatus::from(status),
        tokens_used: tok,
    };
    let sched = |tasks: Vec<aidp_core::plan::TaskState>| aidp_core::plan::ScheduleState {
        stage: "implement".into(),
        plan_path: "/abs/plan.json".into(),
        compiled_at: "t".into(),
        max_parallel: 2,
        width: 2,
        waves: vec![tasks.iter().map(|t| t.id.clone()).collect()],
        serializations: vec![],
        warnings: vec![],
        tasks,
    };
    let mut st = RunState {
        run_id: "r".into(),
        pipeline_path: PathBuf::from("/nonexistent.yaml"),
        pipeline_id: "dev".into(),
        workspace_root: run_dir.clone(),
        run_dir: run_dir.clone(),
        started_at: "t".into(),
        ..Default::default()
    };
    record_plan_schedules(
        &mut st,
        vec![sched(vec![
            mk("T1", "success", Some(500)),
            mk("T2", "pending", None),
        ])],
    )
    .unwrap();
    record_plan_schedules(
        &mut st,
        vec![sched(vec![
            mk("T1", "pending", None),
            mk("T2", "pending", None),
            mk("T3", "pending", None),
        ])],
    )
    .unwrap();
    let tasks = &st.plan_schedules["implement"].tasks;
    assert_eq!(tasks.len(), 3);
    let t1 = tasks.iter().find(|t| t.id == "T1").unwrap();
    assert_eq!(
        t1.status,
        aidp_core::plan::TaskStatus::Success,
        "a known outcome survives a re-emit"
    );
    assert_eq!(t1.tokens_used, Some(500));
    assert_eq!(
        tasks.iter().find(|t| t.id == "T3").unwrap().status,
        aidp_core::plan::TaskStatus::Pending
    );
}

fn implement_only_run(tag: &str) -> RunState {
    let run_dir = scratch_dir();
    RunState {
        run_id: tag.into(),
        pipeline_path: PathBuf::from("/nonexistent.yaml"),
        pipeline_id: "dev".into(),
        workspace_root: run_dir.clone(),
        run_dir: run_dir.clone(),
        started_at: "t".into(),
        stages: vec![StageState {
            name: "implement".into(),
            stage_dir: run_dir.join("implement"),
            ..Default::default()
        }],
        ..Default::default()
    }
}

#[test]
fn reconcile_records_skills_not_loaded_and_keeps_the_stage_successful() {
    let mut st = implement_only_run("skills-own");
    let result = serde_json::json!({
        "status": "done",
        "results": {
            "implement": {
                "status": "needs_review",
                "summary": "skills missing",
                "signals": { "skills_not_loaded": ["rust-dev-aidp", "graphify-context"] }
            }
        }
    });
    reconcile(&mut st, &result).unwrap();
    assert_eq!(
        st.stages[0].skills_not_loaded,
        vec!["rust-dev-aidp".to_string(), "graphify-context".to_string()]
    );
    assert_eq!(st.stages[0].status, StageStatus::Success);
}

#[test]
fn reconcile_folds_plan_task_skills_not_loaded_into_the_parent_stage() {
    let mut st = planned_run("skills-task", &["T1", "T2"]);
    let result = serde_json::json!({
        "status": "done",
        "results": {
            "implement": { "status": "completed", "summary": "merged" },
            "implement#T1": {
                "status": "completed",
                "summary": "ok",
                "signals": { "skills_not_loaded": ["feature-testing-aidp"] }
            },
            "implement#T2": { "status": "completed", "summary": "ok" }
        }
    });
    reconcile(&mut st, &result).unwrap();
    assert_eq!(
        st.stages[0].skills_not_loaded,
        vec!["T1: feature-testing-aidp".to_string()]
    );
}

#[test]
fn render_summary_warns_about_skills_not_loaded() {
    let mut st = implement_only_run("skills-summary");
    st.stages[0].skills_not_loaded = vec!["rust-dev-aidp".into(), "T1: graphify-context".into()];
    let summary = render_summary(&st);
    assert!(
        summary.contains(
            "warning: implement did not load skills: rust-dev-aidp, T1: graphify-context"
        ),
        "got: {summary}"
    );
}
