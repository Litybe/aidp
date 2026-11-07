use super::*;
use crate::run::init;
use aidp_testkit::TempWorkspace;
use std::path::{Path, PathBuf};

const VAULT_PIPE: &str = include_str!("../fixtures/vault-worktree/pipeline.yaml");
const VAULT_DIR: &str = "AIDP";
const OBSIDIAN_DIR: &str = ".obsidian";
const WORKSPACE_DIR: &str = "checkout";
const GIT_DIR: &str = ".git";
const GIT_CONFIG: &str = "config";
const ORIGIN_CONFIG: &str = "[remote \"origin\"]
	url = git@github.com:org/cheat-alerts-service.git
";
const PIPELINE_FILE: &str = "pipe.yaml";
const LINKED_WORKTREE: &str = "linked";
const WORKTREES_DIR: &str = "worktrees";
const PIPELINES_DIR: &str = "Pipelines";

const WORKTREE_DIR: &str = ".worktree";
const FEATURE_KEY: &str = "LOC-62";
const REPO: &str = "cheat-alerts-service";

/// A vault plus a git-backed workspace, the shape real runs happen in.
fn vault_fixture(scratch: &TempWorkspace) -> (PathBuf, Vault) {
    let vault_root = scratch.child(VAULT_DIR);
    std::fs::create_dir_all(vault_root.join(OBSIDIAN_DIR)).unwrap();
    let workspace = scratch.child(WORKSPACE_DIR);
    std::fs::create_dir_all(workspace.join(GIT_DIR)).unwrap();
    std::fs::write(workspace.join(GIT_DIR).join(GIT_CONFIG), ORIGIN_CONFIG).unwrap();
    let vault = Vault {
        root: vault_root,
        source: VaultSource::Flag,
        ambiguity: None,
    };
    (workspace, vault)
}

fn feature_dir(vault_root: &Path) -> PathBuf {
    vault_root.join(PIPELINES_DIR).join(REPO).join(FEATURE_KEY)
}

/// Populates `<feature>/.worktree/` with files, a fake run-state.json and stage-like folders.
fn populate_worktree_dir(feature: &Path) -> PathBuf {
    let worktree = feature.join(WORKTREE_DIR);
    for stage in ["explore", "implement"] {
        std::fs::create_dir_all(worktree.join("20260101-000000-wt01").join(stage)).unwrap();
        std::fs::create_dir_all(worktree.join(stage)).unwrap();
        std::fs::write(worktree.join(stage).join("out.json"), "{}").unwrap();
    }
    std::fs::write(worktree.join("notes.txt"), "scratch").unwrap();
    std::fs::write(
        worktree.join("run-state.json"),
        r#"{"run_id":"ghost-run","pipeline_id":"x"}"#,
    )
    .unwrap();
    std::fs::write(
        worktree.join("20260101-000000-wt01").join("run-state.json"),
        r#"{"run_id":"20260101-000000-wt01","pipeline_id":"x"}"#,
    )
    .unwrap();
    worktree
}

fn all_generated_notes(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let Ok(rd) = std::fs::read_dir(dir) else {
        return found;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        if path.is_dir() {
            found.extend(all_generated_notes(&path));
        } else if path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with('_') && n.ends_with(".md"))
        {
            found.push(path);
        }
    }
    found
}

#[test]
fn worktree_dir_is_absent_from_feature_note_and_receives_no_notes() {
    let scratch = TempWorkspace::new("worktree-dir");
    let (ws, vault) = vault_fixture(&scratch);
    let pipe = ws.join(PIPELINE_FILE);
    std::fs::write(&pipe, VAULT_PIPE).unwrap();
    let state = init(
        &pipe,
        &ws,
        serde_json::json!({"target": FEATURE_KEY}),
        Some(&vault),
    )
    .unwrap();

    let worktree = populate_worktree_dir(&feature_dir(&vault.root));
    crate::obsidian::refresh(&state).unwrap();
    crate::obsidian::ensure_dashboard(&vault.root);
    crate::obsidian::refresh_knowledge(&vault.root);

    let feature_note =
        std::fs::read_to_string(feature_dir(&vault.root).join(format!("_{FEATURE_KEY}.md")))
            .unwrap();
    assert!(feature_note.contains(&state.run_id), "{feature_note}");
    for ghost in [WORKTREE_DIR, "ghost-run", "20260101-000000-wt01"] {
        assert!(
            !feature_note.contains(ghost),
            "feature note must not mention {ghost}:\n{feature_note}"
        );
    }
    let notes = all_generated_notes(&worktree);
    assert!(notes.is_empty(), "notes written under .worktree: {notes:?}");
    assert!(
        !vault.root.join(PIPELINES_DIR).join(WORKTREE_DIR).exists(),
        "no Pipelines/.worktree repo folder"
    );
}

#[test]
fn run_started_from_a_linked_worktree_lands_under_the_main_repo_and_feature() {
    let scratch = TempWorkspace::new("worktree-dir");
    let (ws, vault) = vault_fixture(&scratch);
    let worktree_root = feature_dir(&vault.root)
        .join(WORKTREE_DIR)
        .join(LINKED_WORKTREE);
    let admin_dir = ws.join(GIT_DIR).join(WORKTREES_DIR).join(LINKED_WORKTREE);
    std::fs::create_dir_all(&worktree_root).unwrap();
    std::fs::create_dir_all(&admin_dir).unwrap();
    std::fs::write(
        worktree_root.join(".git"),
        format!("gitdir: {}\n", admin_dir.display()),
    )
    .unwrap();

    assert_eq!(repo_segment(&worktree_root), REPO);

    let pipe = ws.join(PIPELINE_FILE);
    std::fs::write(&pipe, VAULT_PIPE).unwrap();
    let state = init(
        &pipe,
        &worktree_root,
        serde_json::json!({"target": FEATURE_KEY}),
        Some(&vault),
    )
    .unwrap();

    assert_eq!(
        state.run_dir,
        feature_dir(&vault.root).join(&state.run_id),
        "run dir must sit under Pipelines/<repo>/<KEY>/<run_id>/"
    );
    assert_eq!(state.repo.as_deref(), Some(REPO));
    assert!(!vault.root.join(PIPELINES_DIR).join(WORKTREE_DIR).exists());
}
