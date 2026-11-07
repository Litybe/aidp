use aidp_core::pipeline::Placement;
use aidp_core::run::*;

use crate::common;
use common::*;

#[test]
fn runs_are_grouped_under_the_ticket() {
    let ws = scratch_dir();
    std::fs::create_dir_all(&ws).unwrap();
    let p = ws.join("pipe.yaml");
    std::fs::write(&p, GROUPED).unwrap();

    let st = init(&p, &ws, serde_json::json!({"target": "PIXEL-42"}), None).unwrap();
    let pipelines = ws.join(".aidp/pipelines");
    assert_eq!(st.run_dir, pipelines.join("PIXEL-42").join(&st.run_id));
    assert_eq!(st.ticket_root(), pipelines.join("PIXEL-42"));
    assert_eq!(run_dir_for_in(None, &ws, &st.run_id), st.run_dir);
    assert_eq!(load_in(None, &ws, &st.run_id).unwrap().run_dir, st.run_dir);
    let sd = |n: &str| {
        st.stages
            .iter()
            .find(|s| s.name == n)
            .unwrap()
            .stage_dir
            .clone()
    };
    assert_eq!(sd("discover"), st.ticket_root().join("discover"));
    assert_eq!(sd("code"), st.run_dir.join("code"));
    assert_eq!(
        st.stage_dir("discover", Placement::Ticket),
        st.ticket_root().join("discover")
    );
    assert_eq!(
        st.stage_dir("code", Placement::Run),
        st.run_dir.join("code")
    );
}

#[test]
fn run_resolves_from_any_dir_inside_the_workspace() {
    let ws = scratch_dir();
    std::fs::create_dir_all(&ws).unwrap();
    let p = ws.join("pipe.yaml");
    std::fs::write(&p, GROUPED).unwrap();

    let st = init(&p, &ws, serde_json::json!({"target": "PIXEL-42"}), None).unwrap();
    let nested = ws.join("src/Domain");
    std::fs::create_dir_all(&nested).unwrap();
    assert_eq!(run_dir_for_in(None, &nested, &st.run_id), st.run_dir);
    assert_eq!(
        load_in(None, &nested, &st.run_id).unwrap().run_id,
        st.run_id
    );
    assert_eq!(run_dir_for_in(None, &st.run_dir, &st.run_id), st.run_dir);
    assert_eq!(
        load_in(None, &st.run_dir, &st.run_id).unwrap().run_id,
        st.run_id
    );
    let miss = run_dir_for_in(None, &ws, "20990101-000000-zzzz");
    assert_eq!(
        miss,
        ws.join(".aidp/pipelines").join("20990101-000000-zzzz")
    );
}

#[test]
fn target_less_run_falls_back_to_no_ticket() {
    let ws = scratch_dir();
    std::fs::create_dir_all(&ws).unwrap();
    let p = ws.join("pipe.yaml");
    std::fs::write(&p, GROUPED).unwrap();

    let st = init(&p, &ws, serde_json::json!({}), None).unwrap();
    assert_eq!(
        st.run_dir,
        ws.join(".aidp/pipelines")
            .join("_no-ticket")
            .join(&st.run_id)
    );
    assert_eq!(run_dir_for_in(None, &ws, &st.run_id), st.run_dir);
    let st2 = init(&p, &ws, serde_json::json!({"jira_key": "AB-1"}), None).unwrap();
    assert_eq!(st2.ticket_root(), ws.join(".aidp/pipelines").join("AB-1"));
}

#[test]
fn runs_are_grouped_under_the_feature() {
    let ws = scratch_dir();
    std::fs::create_dir_all(&ws).unwrap();
    let p = ws.join("pipe.yaml");
    std::fs::write(&p, GROUPED).unwrap();

    let st = init(
        &p,
        &ws,
        serde_json::json!({"jira_keys": ["PIXEL-2", "PIXEL-1"]}),
        None,
    )
    .unwrap();
    let pipelines = ws.join(".aidp/pipelines");
    assert_eq!(st.ticket_root(), pipelines.join("PIXEL-1+PIXEL-2"));
    assert_eq!(
        st.run_dir,
        pipelines.join("PIXEL-1+PIXEL-2").join(&st.run_id)
    );
    assert_eq!(run_dir_for_in(None, &ws, &st.run_id), st.run_dir);
    assert_eq!(
        st.stage_dir("discover", Placement::Ticket),
        st.ticket_root().join("discover")
    );
}

const KNOWLEDGE_PIPE: &str = r#"
version: 5
id: tech-doc
stages:
  - name: document
    agent: analyzer
    knowledge: true
    outputs: [architecture.md]
  - name: document_transverse
    agent: executor
    knowledge: service
  - name: document_features
    agent: executor
    knowledge: feature
  - name: study
    agent: analyzer
    outputs: [study.md]
  - name: publish
    agent: executor
    needs: [document]
    outputs: [confluence.json]
"#;

#[test]
fn knowledge_stages_write_outside_the_run_and_the_ticket() {
    let (_, ws, vault) = vault_fixture();
    let p = ws.join("pipe.yaml");
    std::fs::write(&p, KNOWLEDGE_PIPE).unwrap();

    let st = init(
        &p,
        &ws,
        serde_json::json!({"target": "PIXEL-7"}),
        Some(&vault),
    )
    .unwrap();

    let knowledge = vault.root.join("Knowledge").join("cheat-alerts-service");
    assert_eq!(st.knowledge_root.as_ref(), Some(&knowledge));
    let sd = |n: &str| {
        st.stages
            .iter()
            .find(|s| s.name == n)
            .unwrap()
            .stage_dir
            .clone()
    };
    assert_eq!(
        sd("document"),
        knowledge,
        "knowledge stages curate the base itself"
    );
    assert_eq!(sd("document_transverse"), knowledge);
    assert_eq!(sd("document_features"), knowledge.join("Features"));
    assert_eq!(sd("study"), st.ticket_root().join("study"));
    assert_eq!(sd("publish"), st.run_dir.join("publish"));
    assert!(
        !sd("document").starts_with(vault.root.join("Pipelines")),
        "the knowledge base must not live under Pipelines/"
    );

    let st2 = init(
        &p,
        &ws,
        serde_json::json!({"target": "PIXEL-8"}),
        Some(&vault),
    )
    .unwrap();
    assert_eq!(st2.knowledge_root, st.knowledge_root);
    assert_ne!(st2.ticket_root(), st.ticket_root());
}

#[test]
fn a_legacy_run_state_without_a_knowledge_root_still_resolves() {
    let ws = scratch_dir();
    std::fs::create_dir_all(&ws).unwrap();
    let p = ws.join("pipe.yaml");
    std::fs::write(&p, GROUPED).unwrap();
    let mut st = init(&p, &ws, serde_json::json!({"target": "PIXEL-9"}), None).unwrap();
    st.knowledge_root = None;
    assert_eq!(st.stage_dir("doc", Placement::Knowledge), st.ticket_root());
}

#[test]
fn run_dir_for_finds_vault_runs_and_still_finds_legacy_ones() {
    let (_, ws, vault) = vault_fixture();
    let p = ws.join("pipe.yaml");
    std::fs::write(&p, VAULT_PIPE).unwrap();

    let vaulted = init(
        &p,
        &ws,
        serde_json::json!({"target": "PIXEL-1"}),
        Some(&vault),
    )
    .unwrap();
    let legacy = init(&p, &ws, serde_json::json!({"target": "PIXEL-0"}), None).unwrap();

    let v = Some(&vault);
    assert_eq!(run_dir_for_in(v, &ws, &vaulted.run_id), vaulted.run_dir);
    assert_eq!(run_dir_for_in(v, &ws, &legacy.run_id), legacy.run_dir);
    assert!(
        legacy.run_dir.starts_with(ws.join(".aidp")),
        "the legacy run stayed in the repo"
    );
    assert!(load_in(v, &ws, &vaulted.run_id).is_ok());
    assert!(
        load_in(v, &ws, &legacy.run_id).is_ok(),
        "pre-v7 runs stay loadable once a vault exists"
    );

    assert_eq!(run_dir_for_in(None, &ws, &legacy.run_id), legacy.run_dir);
}
