use super::*;

#[test]
fn registered_name_wins() {
    let gateway = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let path = project.path().to_string_lossy().replace('\\', "/");
    std::fs::write(
        gateway.path().join("projects.json"),
        format!(r#"[{{"id":"p1","name":"pixelhood","path":"{path}"}}]"#),
    )
    .unwrap();
    let by_name = resolve_project(Some("pixelhood"), Some(gateway.path()), Path::new(".")).unwrap();
    assert_eq!(by_name.key, "pixelhood");
    let by_id = resolve_project(Some("p1"), Some(gateway.path()), Path::new(".")).unwrap();
    assert_eq!(by_id.key, "pixelhood");
    let by_path = resolve_project(
        Some(project.path().to_str().unwrap()),
        Some(gateway.path()),
        Path::new("."),
    )
    .unwrap();
    assert_eq!(by_path.key, "pixelhood");
}

#[test]
fn direct_path_uses_folder_name() {
    let parent = tempfile::tempdir().unwrap();
    let dir = parent.path().join("mygame");
    std::fs::create_dir(&dir).unwrap();
    let found = resolve_project(Some("mygame"), None, parent.path()).unwrap();
    assert_eq!(found.key, "mygame");
    assert_eq!(found.root, dir);
}

const REGISTERED_NAME: &str = "pixelhood";
const WORKTREE_FOLDER: &str = ".worktree";
const MAIN_FOLDER: &str = "mainrepo";

/// A main checkout and a linked worktree whose `.git` file points at it.
fn linked_worktree(parent: &Path) -> (PathBuf, PathBuf) {
    let main = parent.join(MAIN_FOLDER);
    std::fs::create_dir_all(main.join(".git/worktrees/one")).unwrap();
    let worktree = parent.join("GH-1").join(WORKTREE_FOLDER);
    std::fs::create_dir_all(&worktree).unwrap();
    std::fs::write(
        worktree.join(".git"),
        format!(
            "gitdir: {}\n",
            main.join(".git/worktrees/one").to_string_lossy()
        ),
    )
    .unwrap();
    (main, worktree)
}

fn register(gateway: &Path, entry: &str) {
    std::fs::write(gateway.join(PROJECTS_FILE), format!("[{entry}]")).unwrap();
}

#[test]
fn a_linked_worktree_resolves_to_its_registered_project() {
    let gateway = tempfile::tempdir().unwrap();
    let parent = tempfile::tempdir().unwrap();
    let (main, worktree) = linked_worktree(parent.path());
    let path = main.to_string_lossy().replace('\\', "/");
    register(
        gateway.path(),
        &format!(r#"{{"id":"p1","name":"{REGISTERED_NAME}","path":"{path}"}}"#),
    );
    let found = resolve_project(
        Some(worktree.to_str().unwrap()),
        Some(gateway.path()),
        parent.path(),
    )
    .unwrap();
    assert_eq!(found.key, REGISTERED_NAME);
    assert_eq!(comparable(&found.root.to_string_lossy()), comparable(&path));
}

#[test]
fn an_unregistered_worktree_takes_the_name_of_its_main_checkout() {
    let parent = tempfile::tempdir().unwrap();
    let (main, worktree) = linked_worktree(parent.path());
    let found = resolve_project(Some(worktree.to_str().unwrap()), None, parent.path()).unwrap();
    assert_eq!(found.key, MAIN_FOLDER);
    assert_eq!(found.root, main);
}

#[test]
fn the_test_checkout_resolves_to_its_registered_project() {
    let gateway = tempfile::tempdir().unwrap();
    let parent = tempfile::tempdir().unwrap();
    let checkout = parent.path().join("checkout");
    std::fs::create_dir(&checkout).unwrap();
    let path = checkout.to_string_lossy().replace('\\', "/");
    register(
        gateway.path(),
        &format!(
            r#"{{"id":"p1","name":"{REGISTERED_NAME}","path":"Z:/source","testCheckout":"{path}"}}"#
        ),
    );
    let found = resolve_project(
        Some(checkout.to_str().unwrap()),
        Some(gateway.path()),
        parent.path(),
    )
    .unwrap();
    assert_eq!(found.key, REGISTERED_NAME);
}

#[test]
fn registered_keys_are_recognised_by_name_and_id() {
    let gateway = tempfile::tempdir().unwrap();
    register(
        gateway.path(),
        &format!(r#"{{"id":"p1","name":"{REGISTERED_NAME}","path":"Z:/source"}}"#),
    );
    assert!(is_registered(REGISTERED_NAME, Some(gateway.path())));
    assert!(is_registered("p1", Some(gateway.path())));
    assert!(!is_registered(WORKTREE_FOLDER, Some(gateway.path())));
}

#[test]
fn unknown_project_is_not_found() {
    let parent = tempfile::tempdir().unwrap();
    let error = resolve_project(Some("nope"), None, parent.path()).unwrap_err();
    assert!(matches!(error, MemoryError::NotFound(_)));
}
