use super::*;
use aidp_testkit::TempWorkspace;

fn tmp(tag: &str) -> TempWorkspace {
    TempWorkspace::new(tag)
}

#[test]
fn repo_name_from_url_handles_every_remote_shape() {
    assert_eq!(
        repo_name_from_url("git@github.com:org/cheat-alerts-service.git").as_deref(),
        Some("cheat-alerts-service")
    );
    assert_eq!(
        repo_name_from_url("https://github.com/org/cheat-alerts-service.git").as_deref(),
        Some("cheat-alerts-service")
    );
    assert_eq!(
        repo_name_from_url("https://github.com/org/cheat-alerts-service/").as_deref(),
        Some("cheat-alerts-service")
    );
    assert_eq!(
        repo_name_from_url("C:\\CODE\\.Net\\cheat-alerts-service").as_deref(),
        Some("cheat-alerts-service")
    );
}

/// The repo segment must come from the git remote, not the checkout folder,
/// so `.Net/5448/cheat-alerts-service` (a branch-numbered second clone) and
#[test]
fn repo_segment_prefers_the_git_remote_over_the_directory_name() {
    let _temp_ws = tmp("repo-seg");
    let root = _temp_ws.path();
    let ws = root.join("5448").join("checkout-renamed");
    std::fs::create_dir_all(ws.join(".git")).unwrap();
    std::fs::write(
        ws.join(".git").join("config"),
        "[core]\n\tbare = false\n[remote \"origin\"]\n\turl = git@github.com:org/cheat-alerts-service.git\n\tfetch = +refs/heads/*\n",
    )
    .unwrap();
    assert_eq!(repo_segment(&ws), "cheat-alerts-service");

    let plain = root.join("no-git-here");
    std::fs::create_dir_all(&plain).unwrap();
    assert_eq!(repo_segment(&plain), "no-git-here");
}

/// The vault is typically a SIBLING of the repos (C:/CODE/AIDP next to
/// C:/CODE/<repo>), so testing ancestors alone never finds it.
#[test]
fn autodetect_finds_a_sibling_vault() {
    let _temp_ws = tmp("autodetect");
    let root = _temp_ws.path();
    let vault = root.join("AIDP");
    std::fs::create_dir_all(vault.join(".obsidian")).unwrap();
    let ws = root.join("Lambda").join("cheat-actions");
    std::fs::create_dir_all(&ws).unwrap();

    let found = autodetect(&ws).expect("sibling vault found");
    assert_eq!(found.root, vault);
    assert_eq!(found.source, VaultSource::Autodetect);
    assert!(found.ambiguity.is_none());
}

#[test]
fn autodetect_prefers_the_aidp_named_vault_and_records_the_tie() {
    let _temp_ws = tmp("autodetect-tie");
    let root = _temp_ws.path();
    for name in ["AAA-notes", "AIDP"] {
        std::fs::create_dir_all(root.join(name).join(".obsidian")).unwrap();
    }
    let ws = root.join("repo");
    std::fs::create_dir_all(&ws).unwrap();

    let found = autodetect(&ws).expect("vault found");
    assert_eq!(
        found.root,
        root.join("AIDP"),
        "the AIDP-named vault wins the tie"
    );
    assert!(found.ambiguity.as_deref().unwrap().contains("AAA-notes"));
}

#[test]
fn autodetect_returns_none_without_a_vault() {
    let _temp_ws = tmp("autodetect-none");
    let root = _temp_ws.path();
    let ws = root.join("a").join("b");
    std::fs::create_dir_all(&ws).unwrap();
    assert!(autodetect(&ws).is_none());
}

/// The index note can never be a stage's declared artifact: templates declare
/// `plan.md` for a stage named `plan`, and the note must not overwrite it.
#[test]
fn index_note_never_collides_with_a_declared_output() {
    let dir = PathBuf::from("/vault/Pipelines/repo/PIXEL-1/run-x/plan");
    assert_eq!(index_note(&dir).file_name().unwrap(), "_plan.md");
    assert_ne!(index_note(&dir).file_name().unwrap(), "plan.md");
}

#[test]
fn rel_to_is_forward_slashed_and_bounded() {
    let base = PathBuf::from("/vault");
    assert_eq!(
        rel_to(&base, &base.join("Pipelines").join("repo")).as_deref(),
        Some("Pipelines/repo")
    );
    assert!(rel_to(&base, Path::new("/elsewhere/x")).is_none());
}

/// A repo whose `.git` is a directory, plus a vault next to it.
fn state_fixture(tag: &str) -> (PathBuf, PathBuf, Vault) {
    let _temp_ws = tmp(tag);
    let root = _temp_ws.path();
    let ws = root.join("repo");
    std::fs::create_dir_all(ws.join(".git")).unwrap();
    let vault = Vault {
        root: root.join("AIDP"),
        source: VaultSource::Flag,
        ambiguity: None,
    };
    std::fs::create_dir_all(&vault.root).unwrap();
    (root.to_path_buf(), ws, vault)
}

/// The whole point of the move: with a vault configured, the state aidp
/// writes about a repo lands in the vault and NOT in the repo.
#[test]
fn written_state_lands_in_the_vault_not_in_the_repo() {
    let (root, ws, vault) = state_fixture("state-dir");

    assert_eq!(
        repo_state_dir_in(Some(&vault), &ws),
        vault.root.join(PIPELINES_DIR).join("repo").join(STATE_DIR)
    );
    assert_eq!(repo_state_dir_in(None, &ws), ws.join(LEGACY_DIR));

    let marker = repo_state_path_in(Some(&vault), &ws, RUN_IN_FLIGHT_FILE);
    std::fs::create_dir_all(marker.parent().unwrap()).unwrap();
    std::fs::write(&marker, "{}").unwrap();
    assert!(
        !ws.join(LEGACY_DIR).exists(),
        "a run must not create .aidp/ in the target repo"
    );

    let _ = std::fs::remove_dir_all(&root);
}

/// An upgrade must not merely stop writing the stray file - the repo the
/// user is looking at has to actually come out clean.
#[test]
fn a_stray_legacy_file_is_migrated_and_the_empty_folder_pruned() {
    let (root, ws, vault) = state_fixture("state-migrate");
    std::fs::create_dir_all(ws.join(LEGACY_DIR)).unwrap();
    std::fs::write(
        ws.join(LEGACY_DIR).join(CI_AUTOPILOT_STATE_FILE),
        r#"{"version":1}"#,
    )
    .unwrap();

    migrate_repo_state(Some(&vault), &ws).unwrap();
    let moved = repo_state_path_in(Some(&vault), &ws, CI_AUTOPILOT_STATE_FILE);
    assert_eq!(
        std::fs::read_to_string(&moved).unwrap(),
        r#"{"version":1}"#,
        "the ledger's contents survive the move"
    );
    assert!(
        !ws.join(LEGACY_DIR).exists(),
        "an emptied .aidp/ is removed rather than left as a husk"
    );

    let _ = std::fs::remove_dir_all(&root);
}

/// `.aidp/` also holds files the USER wrote. Reclaiming our own must never
/// take those with it, nor delete the folder they live in.
#[test]
fn migration_leaves_user_owned_config_alone() {
    let (root, ws, vault) = state_fixture("state-migrate-config");
    let legacy = ws.join(LEGACY_DIR);
    std::fs::create_dir_all(&legacy).unwrap();
    std::fs::write(legacy.join(RUN_IN_FLIGHT_FILE), "{}").unwrap();
    std::fs::write(legacy.join("dev-target.json"), "{}").unwrap();

    migrate_repo_state(Some(&vault), &ws).unwrap();
    assert!(!legacy.join(RUN_IN_FLIGHT_FILE).exists());
    assert!(
        legacy.join("dev-target.json").is_file(),
        "connection config is the user's, not ours to move"
    );
    assert!(legacy.is_dir(), "a folder still holding user files stays");

    let _ = std::fs::remove_dir_all(&root);
}

/// Both copies present means an older binary wrote the stray one after the
/// vault copy: the vault is authoritative, the stray is simply reclaimed.
#[test]
fn a_vault_copy_wins_over_the_stray_it_replaces() {
    let (root, ws, vault) = state_fixture("state-both");
    let target = repo_state_dir_in(Some(&vault), &ws).join(RUN_IN_FLIGHT_FILE);
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(&target, r#"{"run_id":"vault"}"#).unwrap();
    std::fs::create_dir_all(ws.join(LEGACY_DIR)).unwrap();
    std::fs::write(
        ws.join(LEGACY_DIR).join(RUN_IN_FLIGHT_FILE),
        r#"{"run_id":"stray"}"#,
    )
    .unwrap();

    migrate_repo_state(Some(&vault), &ws).unwrap();
    let resolved = repo_state_path_in(Some(&vault), &ws, RUN_IN_FLIGHT_FILE);
    assert_eq!(resolved, target);
    assert_eq!(
        std::fs::read_to_string(&resolved).unwrap(),
        r#"{"run_id":"vault"}"#
    );
    assert!(!ws.join(LEGACY_DIR).exists());

    let _ = std::fs::remove_dir_all(&root);
}

/// An isolated stage runs from `<repo>/.claude/worktrees/<name>`. A marker
/// written under that name would be invisible to the autopilot, which looks
#[test]
fn a_linked_worktree_shares_the_main_checkouts_state() {
    let (root, ws, vault) = state_fixture("state-worktree");
    let wt = ws.join(".claude").join("worktrees").join("iso");
    std::fs::create_dir_all(&wt).unwrap();
    std::fs::create_dir_all(ws.join(".git").join("worktrees").join("iso")).unwrap();
    std::fs::write(
        wt.join(".git"),
        format!(
            "gitdir: {}",
            ws.join(".git").join("worktrees").join("iso").display()
        ),
    )
    .unwrap();

    assert_eq!(main_worktree_root(&wt), ws);
    assert_eq!(
        repo_state_path_in(Some(&vault), &wt, RUN_IN_FLIGHT_FILE),
        repo_state_path_in(Some(&vault), &ws, RUN_IN_FLIGHT_FILE)
    );

    let _ = std::fs::remove_dir_all(&root);
}

/// Scenario 7: `repo_state_path_in` never touches the filesystem, so a
/// legacy marker is left exactly where an older binary wrote it until
#[test]
fn repo_state_path_is_pure_until_migration_runs() {
    let (root, ws, vault) = state_fixture("state-pure-path");
    let legacy_dir = ws.join(LEGACY_DIR);
    std::fs::create_dir_all(&legacy_dir).unwrap();
    std::fs::write(legacy_dir.join(RUN_IN_FLIGHT_FILE), r#"{"run_id":"x"}"#).unwrap();

    let target = repo_state_path_in(Some(&vault), &ws, RUN_IN_FLIGHT_FILE);
    assert!(
        legacy_dir.join(RUN_IN_FLIGHT_FILE).is_file(),
        "computing the path must not move the legacy file"
    );
    assert!(
        !target.is_file(),
        "computing the path must not create the vault copy either"
    );
    assert!(legacy_dir.is_dir(), "the legacy folder is untouched too");

    migrate_repo_state(Some(&vault), &ws).unwrap();
    assert!(
        !legacy_dir.exists(),
        "migration moves the content into the vault and prunes the empty folder"
    );
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        r#"{"run_id":"x"}"#
    );

    let _ = std::fs::remove_dir_all(&root);
}

/// Dot-prefixed is what keeps `.state` out of Obsidian's explorer AND out of
/// the generated repo MOC, whose scanners all skip dotted entries.
#[test]
fn the_state_folder_is_hidden_from_obsidian() {
    assert!(STATE_DIR.starts_with('.'));
}

#[test]
fn explicit_flag_beats_everything_and_disable_words_force_legacy() {
    let _temp_ws = tmp("resolve");
    let ws = _temp_ws.path();
    let flag = ws.join("explicit-vault");
    let v = resolve(Some(&flag), ws).unwrap();
    assert_eq!(v.source, VaultSource::Flag);
    assert_eq!(v.root, flag);

    assert!(is_disabled("off") && is_disabled("NONE") && is_disabled("0") && is_disabled("false"));
    assert!(!is_disabled("C:/CODE/AIDP"));
}

/// ------------------------------------------- git/info/exclude rules ----

#[test]
fn ensure_repo_ignores_vault_paths_writes_to_plain_repo() {
    let _temp_ws = tmp("ensure-plain");
    let root = _temp_ws.path().to_path_buf();
    let ws = root.join("repo");
    std::fs::create_dir_all(ws.join(".git")).unwrap();

    let result = ensure_repo_ignores_vault_paths(&ws).expect("First call should succeed");
    assert!(result, "First call should return true (file written)");

    let exclude_path = ws.join(".git").join("info").join("exclude");
    assert!(
        exclude_path.exists(),
        "exclude file should have been created"
    );

    let contents = std::fs::read_to_string(&exclude_path).expect("Should read exclude file");
    assert!(
        contents.contains(EXCLUDE_MARKER),
        "Marker should be present in exclude file"
    );
    assert!(
        contents.contains("Pipelines/"),
        "Pipelines/ should be ignored"
    );
    assert!(
        contents.contains("Knowledge/"),
        "Knowledge/ should be ignored"
    );
    assert!(
        contents.contains(".aidp/pipelines/"),
        ".aidp/pipelines/ should be ignored"
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn ensure_repo_ignores_vault_paths_is_idempotent() {
    let _temp_ws = tmp("ensure-idempotent");
    let root = _temp_ws.path().to_path_buf();
    let ws = root.join("repo");
    std::fs::create_dir_all(ws.join(".git")).unwrap();

    let result1 = ensure_repo_ignores_vault_paths(&ws).expect("First call should succeed");
    assert!(result1, "First call should return true");

    let exclude_path = ws.join(".git").join("info").join("exclude");
    let contents1 = std::fs::read_to_string(&exclude_path).expect("Should read exclude file");

    let result2 = ensure_repo_ignores_vault_paths(&ws).expect("Second call should succeed");
    assert!(!result2, "Second call should return false (no-op)");

    let contents2 = std::fs::read_to_string(&exclude_path).expect("Should read exclude file");
    assert_eq!(
        contents1, contents2,
        "File contents should be unchanged after second call"
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn ensure_repo_ignores_vault_paths_handles_no_git() {
    let _temp_ws = tmp("ensure-no-git");
    let root = _temp_ws.path().to_path_buf();
    let ws = root.join("no-repo");
    std::fs::create_dir_all(&ws).unwrap();

    let result = ensure_repo_ignores_vault_paths(&ws).expect("Should not error even without .git");
    assert!(
        !result,
        "Should return false when no .git exists (best-effort)"
    );

    let info_dir = ws.join(".git").join("info");
    assert!(
        !info_dir.exists(),
        "Should not create .git/info/ when not a repository"
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn it_ensure_repo_ignores_vault_paths_handles_linked_worktree() {
    let _temp_ws = tmp("ensure-linked-wt");
    let root = _temp_ws.path().to_path_buf();
    let main_ws = root.join("main");
    let wt = root.join("worktree");

    std::fs::create_dir_all(main_ws.join(".git")).unwrap();

    std::fs::create_dir_all(&wt).unwrap();
    let wt_git_dir = main_ws.join(".git").join("worktrees").join("iso");
    std::fs::create_dir_all(&wt_git_dir).unwrap();
    std::fs::write(wt.join(".git"), format!("gitdir: {}", wt_git_dir.display())).unwrap();

    let result = ensure_repo_ignores_vault_paths(&wt).expect("Should succeed");
    assert!(result, "Should write to main checkout's exclude");

    let exclude_path = main_ws.join(".git").join("info").join("exclude");
    assert!(
        exclude_path.exists(),
        "exclude file should be in the main checkout"
    );

    let contents = std::fs::read_to_string(&exclude_path).expect("Should read exclude file");
    assert!(
        contents.contains("Pipelines/"),
        "Pipelines/ should be in main checkout's exclude"
    );

    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn ensure_repo_ignores_vault_paths_preserves_existing_content() {
    let _temp_ws = tmp("ensure-preserve");
    let root = _temp_ws.path().to_path_buf();
    let ws = root.join("repo");
    std::fs::create_dir_all(ws.join(".git").join("info")).unwrap();

    let exclude_path = ws.join(".git").join("info").join("exclude");
    std::fs::write(&exclude_path, "# My own ignores\n*.swp\n*.bak\n").unwrap();

    let result = ensure_repo_ignores_vault_paths(&ws).expect("Should succeed");
    assert!(result, "Should write new block");

    let contents = std::fs::read_to_string(&exclude_path).expect("Should read exclude file");
    assert!(
        contents.contains("*.swp"),
        "Existing content should be preserved"
    );
    assert!(
        contents.contains("*.bak"),
        "Existing content should be preserved"
    );
    assert!(contents.contains("Pipelines/"), "New rules should be added");
    assert!(
        contents.contains("# aidp: vault-backed"),
        "Marker should be present"
    );

    let _ = std::fs::remove_dir_all(root);
}
