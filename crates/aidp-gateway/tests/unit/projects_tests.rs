use super::Projects;
use aidp_testkit::TempWorkspace;
use std::path::PathBuf;

const GIT_DIR: &str = ".git";
const CODE_DIR: &str = "code";
const RUN_STATE_TEMPLATE: &str = include_str!("../fixtures/projects/run_state.json");
const RUN_STATE_FILE: &str = "run-state.json";
const WORKSPACE_PLACEHOLDER: &str = "WORKSPACE_ROOT";
const RUN_DIR_PLACEHOLDER: &str = "RUN_DIR";
const VAULT_DIR: &str = "vault";
const PIPELINES_DIR: &str = "Pipelines";
const VAULT_REPO: &str = "gamma";
const TICKET_KEY: &str = "LOC-1";
const RUN_ID: &str = "r1";
/// The Windows path separator, replaced by `/` so a path stays a valid JSON string.
const WINDOWS_SEPARATOR: char = '\\';
const STATE_DIR: &str = "state";
const ALPHA: &str = "alpha";
const BETA: &str = "beta";
const PLAIN_DIR: &str = "not-a-repo";

fn repo(root: &TempWorkspace, name: &str) -> PathBuf {
    let path = root.child(&format!("{CODE_DIR}/{name}"));
    std::fs::create_dir_all(path.join(GIT_DIR)).unwrap();
    path
}

#[test]
fn parent_roots_lists_each_checkout_folder_once() {
    let workspace = TempWorkspace::new("projects_parent_roots");
    let projects = Projects::load(&workspace.child(STATE_DIR));
    projects.add(&repo(&workspace, ALPHA).to_string_lossy());
    projects.add(&repo(&workspace, BETA).to_string_lossy());
    assert_eq!(projects.parent_roots(), vec![workspace.child(CODE_DIR)]);
}

#[test]
fn discover_registers_the_sibling_repos_of_a_root() {
    let workspace = TempWorkspace::new("projects_discover");
    let projects = Projects::load(&workspace.child(STATE_DIR));
    projects.add(&repo(&workspace, ALPHA).to_string_lossy());
    repo(&workspace, BETA);
    std::fs::create_dir_all(workspace.child(&format!("{CODE_DIR}/{PLAIN_DIR}"))).unwrap();

    let report = projects.discover(&projects.parent_roots(), None);

    assert_eq!(report.scanned, 2);
    assert_eq!(report.already, 1);
    assert_eq!(report.added.len(), 1);
    assert_eq!(projects.list().len(), 2);
}

/// A vault run of `VAULT_REPO` whose recorded workspace is `workspace`.
fn vault_run(workspace: &TempWorkspace, recorded: &std::path::Path) -> PathBuf {
    let vault = workspace.child(VAULT_DIR);
    let run_dir = vault
        .join(PIPELINES_DIR)
        .join(VAULT_REPO)
        .join(TICKET_KEY)
        .join(RUN_ID);
    std::fs::create_dir_all(&run_dir).unwrap();
    let escape = |path: &std::path::Path| path.to_string_lossy().replace(WINDOWS_SEPARATOR, "/");
    let state = RUN_STATE_TEMPLATE
        .replace(WORKSPACE_PLACEHOLDER, &escape(recorded))
        .replace(RUN_DIR_PLACEHOLDER, &escape(&run_dir));
    std::fs::write(run_dir.join(RUN_STATE_FILE), state).unwrap();
    vault
}

#[test]
fn discover_never_registers_a_ticket_worktree_as_a_project() {
    let workspace = TempWorkspace::new("projects_discover_worktree");
    let worktree = workspace.child(&format!("{TICKET_KEY}/{}", crate::worktrees::WORKTREE_DIR));
    std::fs::create_dir_all(&worktree).unwrap();
    let vault = vault_run(&workspace, &worktree);
    let projects = Projects::load(&workspace.child(STATE_DIR));

    let report = projects.discover(&[], Some(&vault));

    assert!(report.added.is_empty());
    assert_eq!(report.unresolved, vec![VAULT_REPO.to_string()]);
}

#[test]
fn discover_registers_the_checkout_a_vault_run_recorded() {
    let workspace = TempWorkspace::new("projects_discover_vault");
    let checkout = repo(&workspace, VAULT_REPO);
    let vault = vault_run(&workspace, &checkout);
    let projects = Projects::load(&workspace.child(STATE_DIR));

    let report = projects.discover(&[], Some(&vault));

    assert_eq!(report.added.len(), 1);
}
