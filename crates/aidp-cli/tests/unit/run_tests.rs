use super::*;
use aidp_testkit::{fixtures, EnvGuard};

const GATEWAY_DIR_ENV: &str = "AIDP_GATEWAY_DIR";
const TEMPLATE_ID: &str = "dev-exemple";
const CURRENT: &str = ".";
const VAULT_DIR: &str = "vault";
const REPO_DIR: &str = "repo";

fn exec_request() -> ExecRequest {
    ExecRequest {
        template: fixtures::template(TEMPLATE_ID)
            .to_string_lossy()
            .to_string(),
        target: None,
        input: Vec::new(),
        inputs_file: None,
        provider: None,
        approve_gate: Vec::new(),
        foreground: false,
        url: None,
    }
}

/// A context in `cwd`, with the gateway state kept in a temporary directory.
fn launch_with(configure: impl FnOnce(&mut Ctx)) -> (StartNativeRun, tempfile::TempDir) {
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path().join(REPO_DIR);
    std::fs::create_dir_all(&cwd).unwrap();
    let state_dir = temp.path().join("state");
    let _guard = EnvGuard::with(&[
        (GATEWAY_DIR_ENV, Some(state_dir.to_str().unwrap())),
        (AUTO_APPROVE_GATES_ENV, None),
    ]);
    let mut ctx = Ctx::for_test(cwd, None);
    configure(&mut ctx);
    let launch = exec_launch(&ctx, &exec_request()).unwrap();
    (launch, temp)
}

/// `--workspace-root .` reaches the daemon as the absolute current directory.
#[test]
fn a_relative_workspace_root_is_sent_as_an_absolute_path() {
    let (launch, temp) =
        launch_with(|ctx| ctx.globals.workspace_root = Some(PathBuf::from(CURRENT)));
    assert_eq!(launch.workspace, Some(temp.path().join(REPO_DIR)));
}

/// Without `--workspace-root` the workspace is the current directory.
#[test]
fn the_workspace_defaults_to_the_current_directory() {
    let (launch, temp) = launch_with(|_| {});
    assert_eq!(launch.workspace, Some(temp.path().join(REPO_DIR)));
}

/// `--no-vault` is sent along, so the daemon keeps the run in the workspace.
#[test]
fn no_vault_is_sent_to_the_daemon() {
    let (launch, _temp) = launch_with(|ctx| ctx.globals.no_vault = true);
    assert!(launch.no_vault);
    assert_eq!(launch.vault, None);
}

/// A relative `--vault` is made absolute: the daemon does not share this current directory.
#[test]
fn a_relative_vault_is_sent_as_an_absolute_path() {
    let (launch, temp) = launch_with(|ctx| ctx.globals.vault = Some(PathBuf::from(VAULT_DIR)));
    assert_eq!(
        launch.vault,
        Some(temp.path().join(REPO_DIR).join(VAULT_DIR))
    );
    assert!(!launch.no_vault);
}

/// Neither flag leaves the choice to the daemon.
#[test]
fn no_vault_flag_leaves_the_vault_to_the_daemon() {
    let (launch, _temp) = launch_with(|_| {});
    assert!(!launch.no_vault);
    assert_eq!(launch.vault, None);
}
