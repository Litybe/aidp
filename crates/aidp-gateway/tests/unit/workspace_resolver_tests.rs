use super::*;
use aidp_core::process_runner::OsProcessRunner;

const REPO: &str = "repo";

fn project(path: &Path) -> ProjectRef {
    ProjectRef {
        id: "p".to_string(),
        path: path.to_path_buf(),
        repo: REPO.to_string(),
    }
}

fn request(kind: WorkspaceKind) -> WorkspaceRequest {
    WorkspaceRequest {
        kind,
        ticket: None,
        name: None,
        path: None,
    }
}

#[test]
fn scratch_names_follow_the_pattern() {
    for valid in ["a", "0x", "my-scratch-1", &"a".repeat(41)] {
        assert!(is_valid_scratch_name(valid), "{valid}");
    }
    for invalid in ["", "-a", "A", "a_b", "a/b", "a b", "..", &"a".repeat(42)] {
        assert!(!is_valid_scratch_name(invalid), "{invalid}");
    }
}

#[test]
fn project_kind_resolves_to_the_project_path() {
    let dir = tempfile::tempdir().unwrap();
    let workspace = resolve_workspace(
        &OsProcessRunner,
        &request(WorkspaceKind::Project),
        &project(dir.path()),
        &[],
        None,
    )
    .unwrap();
    assert_eq!(workspace.kind, WorkspaceKind::Project);
    assert_eq!(workspace.path, dir.path());
}

#[test]
fn path_outside_roots_is_forbidden() {
    let root = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let error = check_path_allowed(other.path().to_str().unwrap(), &[root.path().to_path_buf()])
        .unwrap_err();
    assert_eq!(error.status, ApiStatus::Forbidden);
    assert_eq!(error.status.status(), 403);
}

#[test]
fn path_inside_roots_is_canonicalized() {
    let root = tempfile::tempdir().unwrap();
    let inner = root.path().join("inner");
    std::fs::create_dir_all(&inner).unwrap();
    let resolved =
        check_path_allowed(inner.to_str().unwrap(), &[root.path().to_path_buf()]).unwrap();
    assert_eq!(resolved, std::fs::canonicalize(&inner).unwrap());
}

#[test]
fn traversal_out_of_a_root_is_forbidden() {
    let root = tempfile::tempdir().unwrap();
    let inner = root.path().join("inner");
    std::fs::create_dir_all(&inner).unwrap();
    let sneaky = format!("{}/../", inner.display());
    let error = check_path_allowed(&sneaky, std::slice::from_ref(&inner)).unwrap_err();
    assert_eq!(error.status, ApiStatus::Forbidden);
}

#[test]
fn sibling_with_common_string_prefix_is_not_inside() {
    let base = tempfile::tempdir().unwrap();
    let root = base.path().join("work");
    let sibling = base.path().join("work-other");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&sibling).unwrap();
    let error = check_path_allowed(sibling.to_str().unwrap(), &[root]).unwrap_err();
    assert_eq!(error.status, ApiStatus::Forbidden);
}

#[test]
fn relative_or_missing_path_is_a_bad_request() {
    let root = tempfile::tempdir().unwrap();
    let roots = [root.path().to_path_buf()];
    assert_eq!(
        check_path_allowed("relative/dir", &roots)
            .unwrap_err()
            .status,
        ApiStatus::BadRequest
    );
    let missing = root.path().join("missing");
    assert_eq!(
        check_path_allowed(missing.to_str().unwrap(), &roots)
            .unwrap_err()
            .status,
        ApiStatus::BadRequest
    );
}

#[test]
fn env_roots_replace_the_defaults() {
    let first = PathBuf::from("a");
    let second = PathBuf::from("b");
    let joined = std::env::join_paths([&first, &second]).unwrap();
    let from_env = allowed_roots(Some(joined), &[PathBuf::from("p")], Some(Path::new("v")));
    assert_eq!(from_env, vec![first, second]);
}

#[test]
fn default_roots_are_projects_and_vault() {
    let defaults = allowed_roots(None, &[PathBuf::from("p")], Some(Path::new("v")));
    assert_eq!(defaults, vec![PathBuf::from("p"), PathBuf::from("v")]);
    let empty = allowed_roots(Some(OsString::new()), &[PathBuf::from("p")], None);
    assert_eq!(empty, vec![PathBuf::from("p")]);
}

#[test]
fn missing_ticket_worktree_is_a_conflict() {
    let vault = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut ticket = request(WorkspaceKind::Ticket);
    ticket.ticket = Some("LOC-9".to_string());
    let error = resolve_workspace(
        &OsProcessRunner,
        &ticket,
        &project(dir.path()),
        &[],
        Some(vault.path()),
    )
    .unwrap_err();
    assert_eq!(error.status, ApiStatus::Conflict);
    assert_eq!(error.status.status(), 409);
}

#[test]
fn scratch_without_vault_is_a_config_error() {
    let dir = tempfile::tempdir().unwrap();
    let mut scratch = request(WorkspaceKind::Scratch);
    scratch.name = Some("ok-name".to_string());
    let error =
        resolve_workspace(&OsProcessRunner, &scratch, &project(dir.path()), &[], None).unwrap_err();
    assert_eq!(error.status, ApiStatus::Config);
}

#[test]
fn scratch_with_invalid_name_is_a_bad_request() {
    let dir = tempfile::tempdir().unwrap();
    let vault = tempfile::tempdir().unwrap();
    let mut scratch = request(WorkspaceKind::Scratch);
    scratch.name = Some("Bad Name".to_string());
    let error = resolve_workspace(
        &OsProcessRunner,
        &scratch,
        &project(dir.path()),
        &[],
        Some(vault.path()),
    )
    .unwrap_err();
    assert_eq!(error.status, ApiStatus::BadRequest);
}
