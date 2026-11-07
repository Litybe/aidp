//! `POST /api/worktrees/ensure` and `/remove` over a real git repository.

use aidp_gateway::http::{serve_on, HttpConfig, ServeHandle};
use aidp_gateway::models::worktree_ensure_response::WorktreeEnsureResponse;
use aidp_gateway::models::worktree_key_request::WorktreeKeyRequest;
use aidp_gateway::models::worktree_remove_response::WorktreeRemoveResponse;
use aidp_gateway::projects::Projects;
use aidp_gateway::worktrees::WIP_REF_PREFIX;
use aidp_testkit::{EnvGuard, TempWorkspace};
use aidp_tickets::model::{Autopilot, IssueType, Priority, Source, Status, Ticket};
use aidp_tickets::store::TicketStore;
use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Command;

const API_BASE: &str = "/api";
const WORKTREES_PREFIX: &str = "/worktrees";
const ENSURE_SEGMENT: &str = "ensure";
const REMOVE_SEGMENT: &str = "remove";
const VAULT_ENV: &str = "AIDP_VAULT";
const PIPELINES_DIR: &str = "Pipelines";
const DEFAULTS_SEEDED_FILE: &str = ".defaults-seeded";
const PROJECT_DIR: &str = "svc";
const FIRST_KEY: &str = "LOC-1";
const SECOND_KEY: &str = "LOC-2";
const MISSING_KEY: &str = "LOC-9";
const HOOK_TOKEN: &str = "hook-secret";
const UI_TOKEN: &str = "ui-secret";
const GIT_PROGRAM: &str = "git";
const AIDP_DIR: &str = ".aidp";
const FLOW_CONFIG_FILE: &str = "flow.json";
const CAP_ONE_CONFIG: &str = r#"{"caps":{"worktrees":1}}"#;
const CAP_REASON: &str = "worktree_cap";
const DIRTY_FILE: &str = "unsaved.txt";
const DIRTY_CONTENT: &str = "work in progress";
const README_FILE: &str = "README.md";
const STATUS_NOT_FOUND: u16 = 404;
const BASE_REF: &str = "refs/remotes/origin/master";
const SHA_LEN: usize = 40;

/// Runs git in `dir`, failing the test on a non-zero exit.
fn git(dir: &Path, args: &[&str]) {
    let status = Command::new(GIT_PROGRAM)
        .arg("-C")
        .arg(dir)
        .args(args)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?} failed");
}

/// A repository with one commit on `master`, an `origin/master` ref and a git identity.
fn seed_project(root: &Path) -> PathBuf {
    let project = root.join(PROJECT_DIR);
    fs::create_dir_all(&project).unwrap();
    git(&project, &["init", "-b", "master"]);
    git(&project, &["config", "user.name", "tester"]);
    git(&project, &["config", "user.email", "tester@example.com"]);
    git(&project, &["config", "commit.gpgsign", "false"]);
    fs::write(project.join(README_FILE), "seed").unwrap();
    git(&project, &["add", "-A"]);
    git(&project, &["commit", "-m", "seed"]);
    git(&project, &["update-ref", BASE_REF, "HEAD"]);
    project
}

/// A local ticket filed under `service`.
fn ticket(key: &str, service: &str) -> Ticket {
    let now = chrono::Utc::now();
    Ticket {
        key: key.to_string(),
        source: Source::Local,
        r#type: IssueType::Task,
        summary: "sample".to_string(),
        status: Status::Todo,
        priority: Priority::Medium,
        priority_rank: 0,
        service: service.to_string(),
        template: None,
        track: None,
        labels: Vec::new(),
        components: Vec::new(),
        epic: None,
        assignee: None,
        created: now,
        updated: now,
        github: None,
        route: None,
        flow: None,
        milestone: None,
        autopilot: Autopilot::default(),
        tags: Vec::new(),
    }
}

/// One daemon over a seeded vault, project and tickets.
struct Fixture {
    _vault: TempWorkspace,
    _state: TempWorkspace,
    _repos: TempWorkspace,
    _env: EnvGuard,
    project: PathBuf,
    handle: Option<ServeHandle>,
    port: u16,
}

impl Fixture {
    fn start(cap_config: Option<&str>) -> Fixture {
        let vault = TempWorkspace::new("gw-wt-vault");
        let state = TempWorkspace::new("gw-wt-state");
        let repos = TempWorkspace::new("gw-wt-repos");
        let env = EnvGuard::set(VAULT_ENV, vault.path().to_str().unwrap());
        let project = seed_project(repos.path());
        if let Some(config) = cap_config {
            let config_dir = project.join(AIDP_DIR);
            fs::create_dir_all(&config_dir).unwrap();
            fs::write(config_dir.join(FLOW_CONFIG_FILE), config).unwrap();
        }
        let registered = Projects::load(state.path()).add(project.to_str().unwrap());
        fs::create_dir_all(vault.path().join(PIPELINES_DIR).join(&registered.name)).unwrap();
        let store = TicketStore::open(vault.path()).unwrap();
        for key in [FIRST_KEY, SECOND_KEY] {
            let mut sample = ticket(key, &registered.name);
            store
                .save(&mut sample, "", &serde_yaml::Mapping::new())
                .unwrap();
        }
        fs::write(state.path().join(DEFAULTS_SEEDED_FILE), "").unwrap();
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let config = HttpConfig {
            state_dir: state.path().to_path_buf(),
            vault_root: vault.path().to_path_buf(),
            hook_token: HOOK_TOKEN.to_string(),
            ui_token: UI_TOKEN.to_string(),
            read_only: false,
            allowed_origins: vec![],
        };
        let handle = serve_on(listener, config).unwrap();
        Fixture {
            _vault: vault,
            _state: state,
            _repos: repos,
            _env: env,
            project,
            handle: Some(handle),
            port,
        }
    }

    fn url(&self, segment: &str) -> String {
        format!(
            "http://127.0.0.1:{}{API_BASE}{WORKTREES_PREFIX}/{segment}",
            self.port
        )
    }

    fn post(&self, segment: &str, key: &str) -> ureq::Response {
        let body = WorktreeKeyRequest {
            key: key.to_string(),
        };
        ureq::post(&self.url(segment))
            .send_json(serde_json::to_value(body).unwrap())
            .unwrap_or_else(|error| error.into_response().unwrap())
    }

    fn ensure(&self, key: &str) -> WorktreeEnsureResponse {
        self.post(ENSURE_SEGMENT, key).into_json().unwrap()
    }

    fn remove(&self, key: &str) -> WorktreeRemoveResponse {
        self.post(REMOVE_SEGMENT, key).into_json().unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.stop();
        }
    }
}

/// A first ensure creates the worktree on disk and reports its base commit.
#[test]
fn ensure_creates_the_worktree() {
    let fixture = Fixture::start(None);

    let response = fixture.ensure(FIRST_KEY);

    let path = PathBuf::from(response.path.expect("path"));
    assert!(path.join(README_FILE).exists());
    assert!(!response.reused);
    assert!(!response.waiting_external);
    assert_eq!(response.base_sha.expect("base sha").len(), SHA_LEN);
}

/// A second ensure of the same ticket reuses the worktree and keeps the path.
#[test]
fn ensure_reuses_an_existing_worktree() {
    let fixture = Fixture::start(None);
    let first = fixture.ensure(FIRST_KEY);

    let second = fixture.ensure(FIRST_KEY);

    assert!(second.reused);
    assert_eq!(second.path, first.path);
}

/// Past the project cap a new ticket waits, while the held ticket still reuses its worktree.
#[test]
fn ensure_waits_when_the_cap_is_reached() {
    let fixture = Fixture::start(Some(CAP_ONE_CONFIG));
    fixture.ensure(FIRST_KEY);

    let blocked = fixture.ensure(SECOND_KEY);
    let reused = fixture.ensure(FIRST_KEY);

    assert!(blocked.waiting_external);
    assert_eq!(blocked.reason.as_deref(), Some(CAP_REASON));
    assert_eq!(blocked.path, None);
    assert!(reused.reused);
    assert!(!reused.waiting_external);
}

/// An unknown ticket is a 404.
#[test]
fn ensure_of_an_unknown_ticket_is_not_found() {
    let fixture = Fixture::start(None);

    let response = fixture.post(ENSURE_SEGMENT, MISSING_KEY);

    assert_eq!(response.status(), STATUS_NOT_FOUND);
}

/// Remove commits unsaved work to a WIP ref, then deletes the worktree.
#[test]
fn remove_saves_work_in_progress_then_deletes() {
    let fixture = Fixture::start(None);
    let path = PathBuf::from(fixture.ensure(FIRST_KEY).path.unwrap());
    fs::write(path.join(DIRTY_FILE), DIRTY_CONTENT).unwrap();

    let response = fixture.remove(FIRST_KEY);

    assert!(response.removed);
    let reference = response.salvaged_ref.expect("salvaged ref");
    assert!(reference.starts_with(WIP_REF_PREFIX));
    assert!(!path.exists());
    git(&fixture.project, &["show-ref", "--verify", &reference]);
}

/// Removing a ticket with no worktree reports nothing removed.
#[test]
fn remove_without_a_worktree_reports_not_removed() {
    let fixture = Fixture::start(None);

    let response = fixture.remove(FIRST_KEY);

    assert!(!response.removed);
    assert_eq!(response.salvaged_ref, None);
}
