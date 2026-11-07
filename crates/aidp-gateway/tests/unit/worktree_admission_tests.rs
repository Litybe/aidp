use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::bus::Bus;
use crate::models::enums::WorkspaceKind;
use crate::projects::Projects;
use crate::sessions::{
    Launch, Origin, OriginKind, SessionManager, StartRequest, WorktreeRefused,
    EVENT_WORKTREE_CREATED,
};
use aidp_core::process_runner::{OsProcessRunner, ProcessRunner, ProcessSpec, SpawnedChild};

const KEY: &str = "LOC-62";
const TAKEN_KEY: &str = "LOC-63";
const BRANCH_ELSEWHERE_SLUG: &str = "branch-checked-out-elsewhere";
const ORIGIN_UI: &str = OriginKind::Ui.as_str();
const ORIGIN_API: &str = OriginKind::Api.as_str();
const POLL_INTERVAL: Duration = Duration::from_millis(20);
const WAIT_LIMIT: Duration = Duration::from_secs(10);

struct Release(Arc<AtomicBool>);

struct BlockedStdout(Arc<AtomicBool>);

impl Read for BlockedStdout {
    fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
        while !self.0.load(Ordering::SeqCst) {
            std::thread::sleep(POLL_INTERVAL);
        }
        Ok(0)
    }
}

struct HeldChild {
    released: Arc<AtomicBool>,
}

impl SpawnedChild for HeldChild {
    fn take_stdout(&mut self) -> Option<Box<dyn Read + Send>> {
        Some(Box::new(BlockedStdout(self.released.clone())))
    }
    fn take_stdin(&mut self) -> Option<Box<dyn Write + Send>> {
        Some(Box::new(std::io::sink()))
    }
    fn id(&self) -> Option<u32> {
        None
    }
    fn wait(&mut self) -> std::io::Result<i32> {
        Ok(0)
    }
    fn try_wait(&mut self) -> std::io::Result<Option<i32>> {
        Ok(self.released.load(Ordering::SeqCst).then_some(0))
    }
    fn kill(&mut self) -> std::io::Result<()> {
        self.released.store(true, Ordering::SeqCst);
        Ok(())
    }
}

#[derive(Default)]
struct HeldSessionRunner {
    spawned: Mutex<Vec<(PathBuf, Release)>>,
}

impl HeldSessionRunner {
    fn cwds(&self) -> Vec<PathBuf> {
        self.spawned
            .lock()
            .unwrap()
            .iter()
            .map(|(cwd, _)| cwd.clone())
            .collect()
    }

    fn release(&self, index: usize) {
        self.spawned.lock().unwrap()[index]
            .1
             .0
            .store(true, Ordering::SeqCst);
    }
}

impl ProcessRunner for HeldSessionRunner {
    fn output(&self, spec: &ProcessSpec) -> std::io::Result<Output> {
        OsProcessRunner.output(spec)
    }

    fn spawn(&self, spec: &ProcessSpec) -> std::io::Result<Box<dyn SpawnedChild>> {
        let released = Arc::new(AtomicBool::new(false));
        self.spawned.lock().unwrap().push((
            spec.cwd.clone().unwrap_or_default(),
            Release(released.clone()),
        ));
        Ok(Box::new(HeldChild { released }))
    }
}

fn git_available() -> bool {
    Command::new("git").arg("--version").output().is_ok()
}

fn run_git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

struct Fixture {
    _root: tempfile::TempDir,
    _vault_env: aidp_testkit::EnvGuard,
    manager: Arc<SessionManager>,
    runner: Arc<HeldSessionRunner>,
    bus: Arc<Bus>,
    project_id: String,
    project_dir: PathBuf,
}

fn fixture() -> Fixture {
    fixture_with_vault(true)
}

fn fixture_with_vault(vault_enabled: bool) -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let origin = root.path().join("origin.git");
    let project_dir = root.path().join("project");
    let vault = root.path().join("vault");
    let state_dir = root.path().join("state");
    for dir in [&origin, &project_dir, &vault, &state_dir] {
        std::fs::create_dir_all(dir).unwrap();
    }
    run_git(&origin, &["init", "--bare", "--initial-branch=master"]);
    run_git(&project_dir, &["init", "--initial-branch=master"]);
    run_git(&project_dir, &["config", "user.email", "t@example.com"]);
    run_git(&project_dir, &["config", "user.name", "t"]);
    run_git(&project_dir, &["config", "commit.gpgsign", "false"]);
    std::fs::write(project_dir.join("a.txt"), "a").unwrap();
    run_git(&project_dir, &["add", "."]);
    run_git(&project_dir, &["commit", "-m", "init"]);
    run_git(
        &project_dir,
        &["remote", "add", "origin", origin.to_str().unwrap()],
    );
    run_git(&project_dir, &["push", "origin", "master"]);

    let vault_setting = if vault_enabled {
        vault.to_str().unwrap()
    } else {
        "off"
    };
    let vault_env = aidp_testkit::EnvGuard::set("AIDP_VAULT", vault_setting);
    let bus = Arc::new(Bus::new());
    let projects = Arc::new(Projects::load(&state_dir));
    let project = projects.add(project_dir.to_str().unwrap());
    let runner = Arc::new(HeldSessionRunner::default());
    let manager =
        SessionManager::load_with_runner(&state_dir, bus.clone(), projects, runner.clone());
    Fixture {
        _root: root,
        _vault_env: vault_env,
        manager,
        runner,
        bus,
        project_id: project.id,
        project_dir,
    }
}

fn request(fx: &Fixture, origin: &str, worktree: Option<&str>) -> StartRequest {
    StartRequest {
        project_id: fx.project_id.clone(),
        engine: Some("prompt".to_string()),
        prompt: Some("hello".to_string()),
        origin: Some(Origin::new(origin)),
        worktree: worktree.map(str::to_string),
        workspace: None,
        title: None,
        external: None,
        provider: None,
        auto_close: Some(false),
        ..Default::default()
    }
}

fn wait_until(what: &str, condition: impl Fn() -> bool) {
    let deadline = Instant::now() + WAIT_LIMIT;
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(POLL_INTERVAL);
    }
}

#[test]
fn it_checkout_and_worktree_sessions_run_concurrently() {
    if !git_available() {
        println!("SKIP: git not found on PATH");
        return;
    }
    let fx = fixture();
    let checkout = fx.manager.start(request(&fx, ORIGIN_UI, None)).unwrap();
    let worktree = fx
        .manager
        .start(request(&fx, ORIGIN_API, Some(KEY)))
        .unwrap();

    let (Launch::Started(checkout), Launch::Started(worktree)) = (checkout, worktree) else {
        panic!("both launches must start immediately");
    };
    let checkout_workspace = checkout.workspace.as_ref().unwrap();
    let worktree_workspace = worktree.workspace.as_ref().unwrap();
    assert_eq!(checkout_workspace.kind, WorkspaceKind::Project);
    assert_eq!(worktree_workspace.kind, WorkspaceKind::Ticket);
    assert_eq!(worktree_workspace.branch.as_deref(), Some("feat/LOC-62"));
    assert_eq!(fx.manager.running_count(), 2);
    assert_eq!(fx.runner.cwds()[1], worktree_workspace.path);
    let events = fx.bus.recent(100, 0);
    assert!(events
        .iter()
        .any(|event| event["type"] == EVENT_WORKTREE_CREATED));
    fx.runner.release(0);
    fx.runner.release(1);
}

#[test]
fn it_ui_launch_never_uses_a_worktree() {
    if !git_available() {
        println!("SKIP: git not found on PATH");
        return;
    }
    let fx = fixture();
    let launch = fx
        .manager
        .start(request(&fx, ORIGIN_UI, Some(KEY)))
        .unwrap();
    let Launch::Started(record) = launch else {
        panic!("expected a started session");
    };
    assert_eq!(
        record.workspace.as_ref().unwrap().kind,
        WorkspaceKind::Project
    );
    assert_eq!(fx.runner.cwds()[0], fx.project_dir);
    fx.runner.release(0);
}

#[test]
fn it_second_session_on_one_worktree_queues_then_drains() {
    if !git_available() {
        println!("SKIP: git not found on PATH");
        return;
    }
    let fx = fixture();
    let first = fx
        .manager
        .start(request(&fx, ORIGIN_API, Some(KEY)))
        .unwrap();
    assert!(matches!(first, Launch::Started(_)));
    let second = fx
        .manager
        .start(request(&fx, ORIGIN_API, Some(KEY)))
        .unwrap();
    assert!(matches!(second, Launch::Queued(_)));
    assert_eq!(fx.manager.queue_depth(), 1);
    assert_eq!(fx.runner.cwds().len(), 1);

    fx.runner.release(0);
    wait_until("the queued launch to start", || {
        fx.manager.queue_depth() == 0 && fx.runner.cwds().len() == 2
    });
    assert_eq!(fx.runner.cwds()[0], fx.runner.cwds()[1]);
    fx.runner.release(1);
}

#[test]
fn it_branch_checked_out_elsewhere_refuses_without_creating_a_session() {
    if !git_available() {
        println!("SKIP: git not found on PATH");
        return;
    }
    let fx = fixture();
    let elsewhere = fx.project_dir.parent().unwrap().join("elsewhere");
    run_git(
        &fx.project_dir,
        &[
            "worktree",
            "add",
            "-b",
            &format!("feat/{TAKEN_KEY}"),
            elsewhere.to_str().unwrap(),
        ],
    );

    let error = match fx.manager.start(request(&fx, ORIGIN_API, Some(TAKEN_KEY))) {
        Ok(_) => panic!("the launch must be refused"),
        Err(error) => error,
    };
    let refused = error
        .downcast_ref::<WorktreeRefused>()
        .expect("typed error");
    assert_eq!(refused.slug, BRANCH_ELSEWHERE_SLUG);
    assert_eq!(fx.manager.running_count(), 0);
    assert_eq!(fx.manager.queue_depth(), 0);
    assert!(fx.runner.cwds().is_empty());
}

#[test]
fn it_missing_vault_runs_in_the_project_with_a_warning() {
    if !git_available() {
        println!("SKIP: git not found on PATH");
        return;
    }
    let fx = fixture_with_vault(false);
    let launch = fx
        .manager
        .start(request(&fx, ORIGIN_API, Some(KEY)))
        .unwrap();
    let Launch::Started(record) = launch else {
        panic!("expected a started session");
    };
    assert_eq!(
        record.workspace.as_ref().unwrap().kind,
        WorkspaceKind::Project
    );
    assert_eq!(
        record.warnings,
        vec![crate::sessions::WARNING_WORKTREE_NO_VAULT.to_string()]
    );
    fx.runner.release(0);
}
