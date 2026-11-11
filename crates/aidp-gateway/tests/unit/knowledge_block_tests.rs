use super::*;

const ENABLED: &str = "version: 5
id: p
settings:
  knowledge: true
stages:
  - name: plan
    agent: planner
  - name: implement
";

const DISABLED: &str = "version: 5
id: p
stages:
  - name: plan
    agent: planner
";

fn state_for(root: &Path, pipeline_text: &str) -> RunState {
    let pipeline_path = root.join("p.yaml");
    std::fs::write(&pipeline_path, pipeline_text).unwrap();
    RunState {
        run_id: "20260101-000000-0000".into(),
        pipeline_path,
        pipeline_id: "p".into(),
        workspace_root: root.join("workspace"),
        run_dir: root.join("run"),
        started_at: "2026-01-01T00:00:00Z".into(),
        ..Default::default()
    }
}

#[test]
fn an_enabled_pipeline_freezes_a_block_per_agent_stage_in_the_given_gateway_only() {
    let root = tempfile::tempdir().unwrap();
    let gateway = root.path().join("gateway");
    std::fs::create_dir_all(root.path().join("workspace")).unwrap();
    let mut state = state_for(root.path(), ENABLED);

    freeze(&gateway, None, &mut state);

    let names: Vec<&str> = state.stage_knowledge.keys().map(String::as_str).collect();
    assert_eq!(names, ["implement", "plan"]);
    assert!(gateway.join("memory").is_dir());
}

#[test]
fn a_pipeline_without_the_setting_freezes_nothing() {
    let root = tempfile::tempdir().unwrap();
    let gateway = root.path().join("gateway");
    let mut state = state_for(root.path(), DISABLED);

    freeze(&gateway, None, &mut state);

    assert!(state.stage_knowledge.is_empty());
    assert!(!gateway.exists());
}

#[test]
fn an_unreadable_pipeline_is_skipped() {
    let root = tempfile::tempdir().unwrap();
    let mut state = state_for(root.path(), ENABLED);
    state.pipeline_path = root.path().join("missing.yaml");

    freeze(&root.path().join("gateway"), None, &mut state);

    assert!(state.stage_knowledge.is_empty());
}
