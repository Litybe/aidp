use aidp_core::pipeline::Pipeline;
use aidp_core::run::*;
use std::path::PathBuf;

use crate::common;
use common::*;

#[test]
fn record_resume_persists_runid_and_gates() {
    let run_dir = scratch_dir();
    let mut st = RunState {
        run_id: "r".into(),
        pipeline_path: PathBuf::from("/nonexistent.yaml"),
        pipeline_id: "dev".into(),
        workspace_root: run_dir.clone(),
        run_dir: run_dir.clone(),
        started_at: "t".into(),
        ..Default::default()
    };
    record_resume(&mut st, Some("wf_1".into()), &["plan".into()], None, None).unwrap();
    record_resume(
        &mut st,
        Some("wf_1".into()),
        &["plan".into(), "pr".into()],
        None,
        None,
    )
    .unwrap();
    assert_eq!(st.workflow_run_id.as_deref(), Some("wf_1"));
    assert_eq!(
        approved_gates(&st),
        vec!["plan".to_string(), "pr".to_string()]
    );
}

#[test]
fn record_resume_tracks_and_clears_pending_gate() {
    let run_dir = scratch_dir();
    let mut st = RunState {
        run_id: "r".into(),
        pipeline_path: PathBuf::from("/nonexistent.yaml"),
        pipeline_id: "dev".into(),
        workspace_root: run_dir.clone(),
        run_dir: run_dir.clone(),
        started_at: "t".into(),
        ..Default::default()
    };
    record_resume(&mut st, Some("wf_9".into()), &[], Some("plan".into()), None).unwrap();
    assert_eq!(st.workflow_run_id.as_deref(), Some("wf_9"));
    assert_eq!(st.pending_gate.as_deref(), Some("plan"));
    assert!(
        approved_gates(&st).is_empty(),
        "a pending gate must not leak into __approved"
    );
    record_resume(&mut st, None, &["plan".into()], None, None).unwrap();
    assert_eq!(st.pending_gate, None);
    assert_eq!(approved_gates(&st), vec!["plan".to_string()]);
    record_resume(&mut st, None, &["pr".into()], Some("pr".into()), None).unwrap();
    assert_eq!(
        st.pending_gate, None,
        "approval in the same call clears pending"
    );
    assert_eq!(
        approved_gates(&st),
        vec!["plan".to_string(), "pr".to_string()]
    );
}

const APPROVE_GATE_FIXTURE: &str = r#"
version: 5
id: fixture-gates
stages:
  - name: discover
    agent: analyzer
  - name: plan
    agent: planner
    needs: [discover]
    gate: "{{ inputs.track == 'deep' }}"
  - name: implement
    agent: executor
    needs: [plan]
    gate: false
  - name: pr
    agent: executor
    needs: [implement]
    gate: true
"#;

fn load_approve_gate_fixture(tag: &str) -> Pipeline {
    let path = scratch_dir().join(format!("approve-gate-{tag}.yaml"));
    std::fs::write(&path, APPROVE_GATE_FIXTURE).unwrap();
    aidp_core::pipeline::load(&path).unwrap()
}

#[test]
fn approve_gate_is_baked_before_the_first_emit() {
    let pipe = load_approve_gate_fixture("baked");
    let (to_approve, unknown) = resolve_approve_gates(&pipe, &["pr".to_string()]);
    assert_eq!(to_approve, vec!["pr".to_string()]);
    assert!(unknown.is_empty());

    let run_dir = scratch_dir();
    let mut st = RunState {
        run_id: "r".into(),
        pipeline_path: PathBuf::from("/nonexistent.yaml"),
        pipeline_id: "fixture-gates".into(),
        workspace_root: run_dir.clone(),
        run_dir: run_dir.clone(),
        started_at: "t".into(),
        ..Default::default()
    };
    record_resume(&mut st, None, &to_approve, None, None).unwrap();
    assert_eq!(approved_gates(&st), vec!["pr".to_string()]);
    assert!(st.pending_gate.is_none());
}

#[test]
fn resolve_approve_gates_reports_an_unknown_name() {
    let pipe = load_approve_gate_fixture("unknown");
    let (to_approve, unknown) =
        resolve_approve_gates(&pipe, &["pr".to_string(), "typo".to_string()]);
    assert_eq!(to_approve, vec!["pr".to_string()]);
    assert_eq!(unknown, vec!["typo".to_string()]);
}

#[test]
fn resolve_approve_gates_star_expands_to_every_declared_gate() {
    let pipe = load_approve_gate_fixture("star");
    let (to_approve, unknown) = resolve_approve_gates(&pipe, &["*".to_string()]);
    assert_eq!(to_approve, vec!["plan".to_string(), "pr".to_string()]);
    assert!(unknown.is_empty());

    let (to_approve, unknown) = resolve_approve_gates(&pipe, &["pr".to_string(), "*".to_string()]);
    assert_eq!(to_approve.len(), 2);
    assert!(to_approve.contains(&"pr".to_string()));
    assert!(to_approve.contains(&"plan".to_string()));
    assert!(unknown.is_empty());
}

#[test]
fn resolve_approve_gates_uses_declaration_not_evaluation() {
    let pipe = load_approve_gate_fixture("declaration");
    let (to_approve, unknown) = resolve_approve_gates(&pipe, &["plan".to_string()]);
    assert_eq!(to_approve, vec!["plan".to_string()]);
    assert!(unknown.is_empty());
    let (star_approve, _) = resolve_approve_gates(&pipe, &["*".to_string()]);
    assert!(star_approve.contains(&"plan".to_string()));

    let (to_approve, unknown) =
        resolve_approve_gates(&pipe, &["implement".to_string(), "discover".to_string()]);
    assert!(to_approve.is_empty());
    assert_eq!(
        unknown,
        vec!["implement".to_string(), "discover".to_string()]
    );
}

const BARE_GATE_RESULT: &str = r#"{"status":"awaiting_gate","gate":"plan","runId":"wf_7"}"#;

fn resume_anchor_state() -> RunState {
    let run_dir = scratch_dir();
    std::fs::create_dir_all(&run_dir).unwrap();
    RunState {
        run_id: "r".into(),
        pipeline_path: PathBuf::from("/nonexistent.yaml"),
        pipeline_id: "dev".into(),
        workspace_root: run_dir.clone(),
        run_dir,
        started_at: "t".into(),
        ..Default::default()
    }
}

fn reload_state(state: &RunState) -> RunState {
    let raw = std::fs::read_to_string(state.run_dir.join("run-state.json")).unwrap();
    serde_json::from_str(&raw).unwrap()
}

#[test]
fn missing_result_file_is_none() {
    let state = resume_anchor_state();
    assert_eq!(read_resume_anchor(&state.run_dir).unwrap(), None);
    assert!(!anchor_not_persisted(&state));
}

#[test]
fn malformed_result_file_errors_with_its_path() {
    let state = resume_anchor_state();
    std::fs::write(state.run_dir.join(WORKFLOW_RESULT_FILE), "not json").unwrap();
    let error = read_resume_anchor(&state.run_dir).unwrap_err();
    assert!(format!("{error:#}").contains(WORKFLOW_RESULT_FILE));
    assert!(!anchor_not_persisted(&state));
}

#[test]
fn recovered_anchor_is_persisted_through_record_resume() {
    let mut state = resume_anchor_state();
    std::fs::write(state.run_dir.join(WORKFLOW_RESULT_FILE), BARE_GATE_RESULT).unwrap();
    assert!(anchor_not_persisted(&state));
    let anchor = read_resume_anchor(&state.run_dir).unwrap().unwrap();
    record_resume(
        &mut state,
        anchor.workflow_run_id,
        &[],
        anchor.pending_gate,
        None,
    )
    .unwrap();
    let reloaded = reload_state(&state);
    assert_eq!(reloaded.workflow_run_id.as_deref(), Some("wf_7"));
    assert_eq!(reloaded.pending_gate.as_deref(), Some("plan"));
    assert!(!anchor_not_persisted(&reloaded));
}

#[test]
fn recovered_anchor_never_reopens_an_approved_gate() {
    let mut state = resume_anchor_state();
    record_resume(&mut state, None, &["plan".into()], None, None).unwrap();
    std::fs::write(state.run_dir.join(WORKFLOW_RESULT_FILE), BARE_GATE_RESULT).unwrap();
    let anchor = read_resume_anchor(&state.run_dir).unwrap().unwrap();
    record_resume(
        &mut state,
        anchor.workflow_run_id,
        &[],
        anchor.pending_gate,
        None,
    )
    .unwrap();
    let reloaded = reload_state(&state);
    assert_eq!(reloaded.pending_gate, None);
    assert_eq!(reloaded.workflow_run_id.as_deref(), Some("wf_7"));
    assert_eq!(approved_gates(&reloaded), vec!["plan".to_string()]);
}
