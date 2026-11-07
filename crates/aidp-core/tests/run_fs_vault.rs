use aidp_core::run::*;
use std::path::PathBuf;
use std::sync::Mutex;

mod common;
use common::*;

static ENV_LOCK: Mutex<()> = Mutex::new(());

#[test]
fn init_writes_the_run_into_the_vault_not_the_repo() {
    let (_, ws, vault) = vault_fixture();
    let p = ws.join("pipe.yaml");
    std::fs::write(&p, VAULT_PIPE).unwrap();

    let st = init(
        &p,
        &ws,
        serde_json::json!({"target": "PIXEL-1234"}),
        Some(&vault),
    )
    .unwrap();

    assert_eq!(
        st.run_dir,
        vault
            .root
            .join("Pipelines")
            .join("cheat-alerts-service")
            .join("PIXEL-1234")
            .join(&st.run_id)
    );
    assert_eq!(
        st.repo.as_deref(),
        Some("cheat-alerts-service"),
        "repo comes from the git remote"
    );
    assert_eq!(st.feature.as_deref(), Some("PIXEL-1234"));
    assert_eq!(st.vault_root.as_ref(), Some(&vault.root));
    assert!(
        !ws.join(".aidp").join("pipelines").exists(),
        "the repo must stay clean of run artefacts"
    );
    let explore = st.stages.iter().find(|s| s.name == "explore").unwrap();
    assert_eq!(explore.stage_dir, st.ticket_root().join("explore"));
    let implement = st.stages.iter().find(|s| s.name == "implement").unwrap();
    assert_eq!(implement.stage_dir, st.run_dir.join("implement"));
}

#[test]
fn init_generates_the_obsidian_note_tree() {
    let (_, ws, vault) = vault_fixture();
    let p = ws.join("pipe.yaml");
    std::fs::write(&p, VAULT_PIPE).unwrap();

    let st = init(
        &p,
        &ws,
        serde_json::json!({"target": "PIXEL-1234"}),
        Some(&vault),
    )
    .unwrap();

    let run_note = st.run_dir.join(format!("_{}.md", st.run_id));
    let feature_moc = st.ticket_root().join("_PIXEL-1234.md");
    let repo_moc = vault
        .root
        .join("Pipelines/cheat-alerts-service/_cheat-alerts-service.md");
    let dashboard = vault.root.join("Pipelines/_Pipelines.md");
    for f in [&run_note, &feature_moc, &repo_moc, &dashboard] {
        assert!(f.is_file(), "missing generated note: {}", f.display());
    }

    let body = std::fs::read_to_string(&run_note).unwrap();
    assert!(
        body.starts_with("---\n"),
        "run note must open with YAML frontmatter"
    );
    assert!(body.contains("aidp: run"));
    assert!(body.contains("repo: cheat-alerts-service"));
    assert!(body.contains("feature: PIXEL-1234"));
    assert!(
        body.contains("#[[Pipelines/cheat-alerts-service/PIXEL-1234/_PIXEL-1234|PIXEL-1234]]")
            || body
                .contains("[[Pipelines/cheat-alerts-service/PIXEL-1234/_PIXEL-1234|PIXEL-1234]]"),
        "the run note must link back to its feature MOC:\n{body}"
    );

    let moc = std::fs::read_to_string(&feature_moc).unwrap();
    assert!(
        moc.contains(&st.run_id),
        "the feature MOC must list the run"
    );
}

#[test]
fn reconcile_folds_agent_prose_into_the_stage_note() {
    let (_, ws, vault) = vault_fixture();
    let p = ws.join("pipe.yaml");
    std::fs::write(&p, VAULT_PIPE).unwrap();
    let mut st = init(
        &p,
        &ws,
        serde_json::json!({"target": "PIXEL-9"}),
        Some(&vault),
    )
    .unwrap();
    let impl_dir = st.run_dir.join("implement");
    std::fs::create_dir_all(&impl_dir).unwrap();
    let changed = impl_dir
        .join("changed-files.json")
        .to_string_lossy()
        .into_owned();

    reconcile(
        &mut st,
        &serde_json::json!({
            "status": "done",
            "results": {
                "implement": {
                    "status": "completed",
                    "summary": "Added the handler and its tests.",
                    "follow_ups": ["Ask the data team about retention."],
                    "outputs": { "changed": changed },
                    "metrics": { "tokens_used": 4242 }
                }
            }
        }),
    )
    .unwrap();

    let stage = st.stages.iter().find(|s| s.name == "implement").unwrap();
    assert_eq!(
        stage.summary.as_deref(),
        Some("Added the handler and its tests.")
    );
    assert_eq!(
        stage.follow_ups,
        vec!["Ask the data team about retention.".to_string()]
    );

    let note = std::fs::read_to_string(st.run_dir.join("implement").join("_implement.md")).unwrap();
    assert!(note.contains("Added the handler and its tests."));
    assert!(note.contains("agent: executor"));
    assert!(note.contains("tokens: 4242"));

    let run_note = std::fs::read_to_string(st.run_dir.join(format!("_{}.md", st.run_id))).unwrap();
    assert!(
        run_note.contains("Ask the data team about retention."),
        "follow-ups surface on the run note"
    );
}

#[test]
fn no_vault_keeps_the_legacy_layout_and_writes_no_notes() {
    let (_, ws, _vault) = vault_fixture();
    let p = ws.join("pipe.yaml");
    std::fs::write(&p, VAULT_PIPE).unwrap();

    let st = init(&p, &ws, serde_json::json!({"target": "PIXEL-7"}), None).unwrap();

    assert_eq!(
        st.run_dir,
        ws.join(".aidp/pipelines").join("PIXEL-7").join(&st.run_id)
    );
    assert!(st.vault_root.is_none());
    assert!(
        !st.run_dir.join(format!("_{}.md", st.run_id)).exists(),
        "no notes without a vault"
    );
}

const SNAPSHOT_PIPE: &str = r#"
version: 5
id: loc44-knowledge-snapshot
stages:
  - name: discover
    agent: analyzer
  - name: knowledge_sync
    agent: documenter
    knowledge: true
    needs: [discover]
"#;

#[test]
fn knowledge_snapshot_keeps_an_unrelated_readers_prompt_identical_across_a_knowledge_write() {
    let ws = scratch_dir();
    std::fs::create_dir_all(&ws).unwrap();
    let p = ws.join("pipe.yaml");
    std::fs::write(&p, SNAPSHOT_PIPE).unwrap();

    let mut st = init(&p, &ws, serde_json::json!({}), None).unwrap();
    let kdir = st.knowledge_dir();
    std::fs::create_dir_all(&kdir).unwrap();
    std::fs::write(kdir.join("overview.md"), "# Overview\n").unwrap();

    ensure_knowledge_snapshot(&mut st).unwrap();
    let p1 = aidp_core::prompt::build_prompt_text(
        &aidp_core::pipeline::load(&st.pipeline_path).unwrap(),
        &st,
        "discover",
    )
    .unwrap();
    assert!(
        p1.contains("overview.md"),
        "sanity: the reader's prompt must list the note that existed at snapshot time:\n{p1}"
    );

    std::fs::write(kdir.join("operations.md"), "# Operations\n").unwrap();

    ensure_knowledge_snapshot(&mut st).unwrap();
    let p2 = aidp_core::prompt::build_prompt_text(
        &aidp_core::pipeline::load(&st.pipeline_path).unwrap(),
        &st,
        "discover",
    )
    .unwrap();

    assert_eq!(
        p1, p2,
        "an unrelated reader's prompt (and therefore its Workflow cache key) must not \
             change just because knowledge_sync wrote to the base between two emits of the \
             same run — resume would re-run `discover` instead of replaying it from cache"
    );
    assert!(
        !p2.contains("operations.md"),
        "the frozen snapshot must not surface a note written after it was captured:\n{p2}"
    );

    let mut unsnapshotted = st.clone();
    unsnapshotted.knowledge_snapshot = None;
    let live = aidp_core::prompt::build_prompt_text(
        &aidp_core::pipeline::load(&unsnapshotted.pipeline_path).unwrap(),
        &unsnapshotted,
        "discover",
    )
    .unwrap();
    assert!(
        live.contains("operations.md"),
        "a live (unsnapshotted) read must see the new note — otherwise this test cannot \
             tell a fixed prompt.rs from a prompt.rs that never wrote anything to disk:\n{live}"
    );
}

#[test]
fn knowledge_writer_section_keeps_its_frozen_decision_across_a_note_write() {
    let ws = scratch_dir();
    std::fs::create_dir_all(&ws).unwrap();
    let p = ws.join("pipe.yaml");
    std::fs::write(&p, SNAPSHOT_PIPE).unwrap();

    let mut st = init(
        &p,
        &ws,
        serde_json::json!({"feature": "Auto-exclusion temporaire du joueur"}),
        None,
    )
    .unwrap();
    let features_dir = aidp_core::knowledge::features_dir(&st.knowledge_dir());
    std::fs::create_dir_all(&features_dir).unwrap();

    ensure_knowledge_snapshot(&mut st).unwrap();
    let p1 = aidp_core::prompt::build_prompt_text(
        &aidp_core::pipeline::load(&st.pipeline_path).unwrap(),
        &st,
        "knowledge_sync",
    )
    .unwrap();
    assert!(
        p1.contains("this is a NEW feature"),
        "with no matching note yet, the writer must be told to create one:
{p1}"
    );

    std::fs::write(
        features_dir.join("auto-exclusion-temporaire.md"),
        "---
aidp: knowledge-feature
feature: Auto-exclusion temporaire du joueur
slug: auto-exclusion-temporaire
status: active
---

# Auto-exclusion temporaire
",
    )
    .unwrap();

    ensure_knowledge_snapshot(&mut st).unwrap();
    let p2 = aidp_core::prompt::build_prompt_text(
        &aidp_core::pipeline::load(&st.pipeline_path).unwrap(),
        &st,
        "knowledge_sync",
    )
    .unwrap();
    assert_eq!(
            p1, p2,
            "the writer's prompt (its Workflow cache key) must not change once it has written              the note it resolved — resume would re-run it, and `plan` after it"
        );

    let mut unsnapshotted = st.clone();
    unsnapshotted.knowledge_snapshot = None;
    let live = aidp_core::prompt::build_prompt_text(
        &aidp_core::pipeline::load(&unsnapshotted.pipeline_path).unwrap(),
        &unsnapshotted,
        "knowledge_sync",
    )
    .unwrap();
    assert!(
        live.contains("EXISTING feature"),
        "a live read must resolve the note just written:
{live}"
    );
}

/// An invalid `run-state.json` fails `load_in` with a message naming the file.
/// The error chain still carries the underlying `serde_json::Error`.
#[test]
fn load_in_on_invalid_run_state_names_the_path_and_keeps_the_json_error_in_the_chain() {
    let dir = scratch_dir();
    let run_id = "20260101-000000-badj";
    let run_dir = dir.join(".aidp/pipelines").join(run_id);
    std::fs::create_dir_all(&run_dir).unwrap();
    let state_path = run_dir.join("run-state.json");
    std::fs::write(&state_path, "{ not valid json").unwrap();

    let error = load_in(None, &dir, run_id).expect_err("invalid run state must not load");
    let rendered = format!("{error:#}");
    assert!(rendered.contains("run-state.json"), "{rendered}");
    assert!(error.chain().nth(1).is_some(), "{rendered}");
    assert!(
        error
            .chain()
            .any(|c| c.downcast_ref::<serde_json::Error>().is_some()),
        "{rendered}"
    );
}

#[test]
fn load_in_with_no_vault_ignores_aidp_vault_entirely() {
    let _guard = ENV_LOCK.lock().unwrap();
    let ws = scratch_dir();
    std::fs::create_dir_all(&ws).unwrap();
    let trap = scratch_dir().join("vault-trap");

    let prev = std::env::var_os("AIDP_VAULT");
    std::env::set_var("AIDP_VAULT", &trap);

    let p = ws.join("pipe.yaml");
    std::fs::write(&p, GROUPED).unwrap();
    let st = init(&p, &ws, serde_json::json!({"target": "PIXEL-5"}), None).unwrap();

    assert_eq!(
        st.run_dir,
        ws.join(".aidp/pipelines").join("PIXEL-5").join(&st.run_id)
    );
    assert_eq!(
        run_dir_for_in(None, &ws, &st.run_id),
        st.run_dir,
        "a legacy lookup must resolve without consulting AIDP_VAULT"
    );
    assert_eq!(load_in(None, &ws, &st.run_id).unwrap().run_id, st.run_id);
    assert!(
        !trap.exists(),
        "the vault named by AIDP_VAULT must never be created by a legacy-only lookup"
    );

    match prev {
        Some(v) => std::env::set_var("AIDP_VAULT", v),
        None => std::env::remove_var("AIDP_VAULT"),
    }
}

#[test]
fn save_state_writes_run_state_json_and_no_notes() {
    let (_, ws, vault) = vault_fixture();
    let p = ws.join("pipe.yaml");
    std::fs::write(&p, VAULT_PIPE).unwrap();
    let st = init(
        &p,
        &ws,
        serde_json::json!({"target": "PIXEL-3"}),
        Some(&vault),
    )
    .unwrap();

    let before: Vec<PathBuf> = std::fs::read_dir(&st.run_dir)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("md"))
        .collect();

    save_state(&st).unwrap();

    assert!(
        st.run_dir.join("run-state.json").is_file(),
        "save_state must write run-state.json"
    );
    let after: Vec<PathBuf> = std::fs::read_dir(&st.run_dir)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("md"))
        .collect();
    assert_eq!(
        before, after,
        "save_state must create no note — that is `save`'s (obsidian::refresh) job"
    );
}
