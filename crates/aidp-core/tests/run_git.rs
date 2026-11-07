use aidp_core::run::*;
use std::path::PathBuf;

use crate::common;
use common::scratch_dir;

#[test]
fn reconcile_folds_workflow_result() {
    let mk = |name: &str| StageState {
        name: name.into(),
        stage_dir: PathBuf::from(format!("/ws/{name}")),
        ..Default::default()
    };
    let run_dir = scratch_dir();
    std::fs::create_dir_all(&run_dir).unwrap();
    let git = |args: &[&str]| {
        assert!(std::process::Command::new("git")
            .args(args)
            .current_dir(&run_dir)
            .status()
            .expect("git must be on PATH for this test")
            .success());
    };
    git(&["init", "-q"]);
    git(&["config", "user.email", "test@example.com"]);
    git(&["config", "user.name", "aidp tests"]);
    std::fs::write(run_dir.join("seed.txt"), "seed").unwrap();
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "chore: seed"]);
    let base_sha = aidp_core::gitlog::head(&run_dir).expect("HEAD must resolve on a fresh repo");

    let mut st = RunState {
        run_id: "r".into(),
        pipeline_path: PathBuf::from("/nonexistent.yaml"),
        pipeline_id: "dev".into(),
        workspace_root: run_dir.clone(),
        run_dir: run_dir.clone(),
        started_at: "t".into(),
        base_sha: Some(base_sha.clone()),
        stages: vec![mk("code"), mk("review"), mk("study")],
        ..Default::default()
    };
    let result = serde_json::json!({
        "status": "done",
        "results": {
            "code": {
                "status": "completed", "summary": "did it",
                "outputs": { "changed-files": "/ws/code/changed-files.json" },
                "metrics": { "tokens_used": 4200, "duration_ms": 8000 }
            },
            "review": { "status": "needs_review", "summary": "see findings" }
        }
    });
    reconcile(&mut st, &result).unwrap();
    let code = st.stages.iter().find(|s| s.name == "code").unwrap();
    assert_eq!(code.status, StageStatus::Success);
    assert_eq!(code.tokens_used, Some(4200));
    assert_eq!(code.duration_ms, Some(8000));
    assert_eq!(
        code.outputs.get("changed-files").unwrap(),
        &PathBuf::from("/ws/code/changed-files.json")
    );
    assert_eq!(
        st.stages
            .iter()
            .find(|s| s.name == "review")
            .unwrap()
            .status,
        StageStatus::Success
    );
    assert_eq!(
        st.stages.iter().find(|s| s.name == "study").unwrap().status,
        StageStatus::Skipped
    );
    assert!(
        code.started_at.is_none() && code.finished_at.is_none(),
        "no fabricated dates on code"
    );
    let review = st.stages.iter().find(|s| s.name == "review").unwrap();
    assert!(
        review.started_at.is_none() && review.finished_at.is_none() && review.duration_ms.is_none(),
        "no fabricated dates/duration on a stage without runtime metrics"
    );
    assert_eq!(st.last_workflow_status.as_deref(), Some("done"));

    assert_eq!(
        st.base_sha.as_deref(),
        Some(base_sha.as_str()),
        "base_sha set before reconcile must survive it untouched"
    );
    assert_eq!(
        st.head_sha_end.as_deref(),
        Some(base_sha.as_str()),
        "head_sha_end must be populated from the current HEAD"
    );
    assert!(
        st.commits.contains(&base_sha),
        "commits must include the seed commit: {:?}",
        st.commits
    );
    assert!(st.scope_violations.is_empty());
    assert!(
        code.verification
            .iter()
            .any(|s| s.contains("missing-or-unreadable")),
        "got: {:?}",
        code.verification
    );

    reconcile(&mut st, &result).unwrap();
    assert_eq!(
        st.stages
            .iter()
            .find(|s| s.name == "code")
            .unwrap()
            .tokens_used,
        Some(4200)
    );
}

#[test]
fn reconcile_skips_git_anchoring_and_scope_verify_on_a_paused_result() {
    let run_dir = scratch_dir();
    std::fs::create_dir_all(&run_dir).unwrap();
    let git = |args: &[&str]| {
        assert!(std::process::Command::new("git")
            .args(args)
            .current_dir(&run_dir)
            .status()
            .expect("git must be on PATH for this test")
            .success());
    };
    git(&["init", "-q"]);
    git(&["config", "user.email", "test@example.com"]);
    git(&["config", "user.name", "aidp tests"]);
    std::fs::write(run_dir.join("seed.txt"), "seed").unwrap();
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "chore: seed"]);
    let base_sha = aidp_core::gitlog::head(&run_dir).expect("HEAD must resolve on a fresh repo");

    let pipeline_path = run_dir.join("pipe.yaml");
    std::fs::write(
            &pipeline_path,
            "version: 5\nid: freeze-test\nsettings:\n  verify_scope: true\nstages:\n  - { name: code, agent: executor, scope: { mode: repo_wide } }\n",
        )
        .unwrap();

    let sentinel_head = "cccccccccccccccccccccccccccccccccccccccc".to_string();
    let mut st = RunState {
        run_id: "r".into(),
        pipeline_path,
        pipeline_id: "dev".into(),
        workspace_root: run_dir.clone(),
        run_dir: run_dir.clone(),
        started_at: "t".into(),
        base_sha: Some(base_sha.clone()),
        head_sha_end: Some(sentinel_head.clone()),
        scope_violations: vec![aidp_core::scope::Violation {
            path: "src/Sentinel.cs".into(),
            reason: "sentinel — must survive a paused reconcile untouched".into(),
        }],
        stages: vec![StageState {
            name: "code".into(),
            stage_dir: run_dir.clone(),
            ..Default::default()
        }],
        ..Default::default()
    };

    let paused = serde_json::json!({
        "status": "awaiting_gate",
        "gate": "pr",
        "results": { "code": { "status": "completed" } }
    });
    reconcile(&mut st, &paused).unwrap();
    assert_eq!(
        st.head_sha_end.as_deref(),
        Some(sentinel_head.as_str()),
        "head_sha_end must not be recomputed while the run is only paused"
    );
    assert_eq!(
        st.scope_violations.len(),
        1,
        "scope_violations must not be recomputed while the run is only paused: {:?}",
        st.scope_violations
    );
    assert_eq!(st.scope_violations[0].path, "src/Sentinel.cs");

    let done = serde_json::json!({
        "status": "done",
        "results": { "code": { "status": "completed" } }
    });
    reconcile(&mut st, &done).unwrap();
    assert_eq!(
        st.head_sha_end.as_deref(),
        Some(base_sha.as_str()),
        "head_sha_end must be recomputed to the real HEAD once the run is finished"
    );
    assert!(
        st.scope_violations.is_empty(),
        "scope_violations must actually be recomputed once finished — a repo_wide stage \
             accepts everything, so the real recompute finds none: {:?}",
        st.scope_violations
    );
}
