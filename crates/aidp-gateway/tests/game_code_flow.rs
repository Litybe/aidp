//! End-to-end `game-code` (dev lane) and `game-test-code` (test lane) flows: the real flow engine, the real native run executor and the real
//! Unity editor lock, with a scripted provider writing each pipeline's `result.json`, and the
//! `aidp` verbs the actions spawn answered by [`FakeAidp`] against the GitHub mock server.

use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::native_runs::fake_provider::{Behaviour, Call, ScriptedProvider, PROVIDER_ID};
use aidp_agents::ProvidersConfig;
use aidp_core::clock::SystemClock;
use aidp_core::id::SystemIdGen;
use aidp_core::process_runner::{ProcessRunner, ProcessSpec, ScriptedProcessRunner, SpawnedChild};
use aidp_flows::{FlowDef, FlowState, FlowStatus, StepStatus};
use aidp_gateway::bus::Bus;
use aidp_gateway::flows::executor::Ports;
use aidp_gateway::flows::lock::SessionLock;
use aidp_gateway::flows::registry::{AlwaysAllow, ManagerSessions, Registry};
use aidp_gateway::flows::{FlowEngine, FlowEngineConfig};
use aidp_gateway::launcher::RunLauncher;
use aidp_gateway::models::flow_execution_policy::FlowExecutionPolicy;
use aidp_gateway::models::{LocalLock, TicketFlowRequest};
use aidp_gateway::native_runs::{NativeDeps, NativeRunManager};
use aidp_gateway::projects::Projects;
use aidp_gateway::sessions::SessionManager;
use aidp_integrations::mock::{serve_on, MockHandle};
use aidp_testkit::{EnvGuard, GitFixture, TempWorkspace};
use serde_json::{json, Value};

const DEV_FLOW: &str = include_str!("fixtures/game_flows/game-code.yaml");
const TEST_FLOW: &str = include_str!("fixtures/game_flows/game-test-code.yaml");
const SKILL_ROOT: &str = "tests/fixtures/game_flows";
const REPO: &str = "o/r";
const SETTLE: Duration = Duration::from_secs(30);
const POLL: Duration = Duration::from_millis(20);
const PR_NUMBER: u64 = 7;
const MAIN_BRANCH: &str = "main";
const README_FILE: &str = "README.md";
const README_CONTENTS: &str = "game
";
const GAME_IDENTITY_NAME: &str = "game";
const GAME_IDENTITY_EMAIL: &str = "game@example.test";
const GAME_FIXTURE: GitFixture<'static> = GitFixture {
    files: &[(README_FILE, README_CONTENTS)],
    empty_directories: &[],
    branch: Some(MAIN_BRANCH),
    user_name: GAME_IDENTITY_NAME,
    user_email: GAME_IDENTITY_EMAIL,
    autocrlf_off: true,
};
/// The pull request the dev lane opened: the test lane has no `pr.json` of its own and reads it live.
struct DevPr;

impl aidp_gateway::models::PrStatusSource for DevPr {
    fn status(&self, _key: &str) -> Option<Value> {
        Some(
            json!({ "number": PR_NUMBER, "url": format!("https://github.com/{REPO}/pull/{PR_NUMBER}"),
                     "repo": REPO, "head": { "sha": HEAD_SHA } }),
        )
    }
}

fn registry_with_pr() -> Registry {
    let mut registry = Registry::with_builtins();
    registry.set_pr_status_source(Arc::new(DevPr));
    registry
}

const HEAD_SHA: &str = "0123abcd0123abcd0123abcd0123abcd0123abcd";
const COST_LINE: &str = "Coût : 1,2 M tokens";
const UNITY_REASON: &str = "resource:unity-editor";
const ENTRY_MERGE: &str = "merge";
const ENTRY_REVIEW: &str = "review";

/// The `aidp` CLI as the flow actions see it. Each verb does what the real one does: label and
/// comment write on the GitHub mock (the comment body is read from `--body-file`, as the real
/// verb does), PR verbs are recorded. A `--cost-footer` appends a fixed line.
struct FakeAidp {
    mock: String,
    project: PathBuf,
    vault: PathBuf,
    /// The main checkout has local changes: `git-checkout` reports `blocked`.
    dirty: AtomicBool,
    /// `pr-merge` reports a rebase without merging (`merged: false`).
    merge_stalls: AtomicBool,
    issues: BTreeMap<String, u64>,
    /// Every verb in call order: `worktree:<key>`, `branch:<key>`, `worktree-remove:<key>`, `checkout:<key>`, `restore:<key>`, `label:<aidp:x>`, `comment:<key>`, `pr-ready:<n>`, `pr-merge:<n>:<sha>`.
    verbs: Mutex<Vec<String>>,
    /// The `aidp:*` labels the issue showed after each label call.
    snapshots: Mutex<Vec<Vec<String>>>,
}

struct Finished {
    stdout: Vec<u8>,
    code: i32,
}

impl SpawnedChild for Finished {
    fn take_stdout(&mut self) -> Option<Box<dyn Read + Send>> {
        Some(Box::new(Cursor::new(std::mem::take(&mut self.stdout))))
    }
    fn take_stdin(&mut self) -> Option<Box<dyn Write + Send>> {
        None
    }
    fn id(&self) -> Option<u32> {
        None
    }
    fn wait(&mut self) -> std::io::Result<i32> {
        Ok(self.code)
    }
    fn try_wait(&mut self) -> std::io::Result<Option<i32>> {
        Ok(Some(self.code))
    }
    fn kill(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl FakeAidp {
    fn url(&self, number: u64, tail: &str) -> String {
        format!("{}/github/repos/{REPO}/issues/{number}{tail}", self.mock)
    }

    fn issue_labels(&self, number: u64) -> Vec<String> {
        let issue: Value = ureq::get(&self.url(number, ""))
            .call()
            .expect("issue is seeded")
            .into_json()
            .unwrap();
        issue["labels"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|label| label["name"].as_str().map(str::to_string))
            .filter(|name| name.starts_with("aidp:"))
            .collect()
    }

    fn label(&self, key: &str, state: &str) -> (i32, Value) {
        let number = self.issues[key];
        let wanted = if state.starts_with("aidp:") {
            state.to_string()
        } else {
            format!("aidp:{state}")
        };
        let before = self.issue_labels(number);
        let removed: Vec<String> = before
            .iter()
            .filter(|name| **name != wanted)
            .cloned()
            .collect();
        for name in &removed {
            let encoded = name.replace(':', "%3A");
            ureq::delete(&self.url(number, &format!("/labels/{encoded}")))
                .call()
                .expect("label removed");
        }
        let changed = !before.contains(&wanted);
        if changed {
            ureq::post(&self.url(number, "/labels"))
                .send_json(json!({ "labels": [wanted] }))
                .expect("label added");
        }
        self.verbs.lock().unwrap().push(format!("label:{wanted}"));
        self.snapshots
            .lock()
            .unwrap()
            .push(self.issue_labels(number));
        (
            0,
            json!({ "key": key, "label": wanted, "changed": changed, "removed": removed }),
        )
    }

    fn comment(&self, key: &str, args: &[&str]) -> (i32, Value) {
        let Some(path) = flag_value(args, "--body-file") else {
            return (2, json!("ticket comment needs --body-file"));
        };
        let mut text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) => return (1, json!(format!("cannot read `{path}`: {error}"))),
        };
        let footer = args.contains(&"--cost-footer");
        if footer {
            text = format!("{}\n\n{COST_LINE}\n", text.trim_end());
        }
        ureq::post(&self.url(self.issues[key], "/comments"))
            .send_json(json!({ "body": text }))
            .expect("comment posted");
        self.verbs.lock().unwrap().push(format!("comment:{key}"));
        (
            0,
            json!({ "key": key, "url": "mock", "costFooter": footer }),
        )
    }

    fn dispatch(&self, args: &[String]) -> (i32, Value) {
        let args: Vec<&str> = args.iter().map(String::as_str).collect();
        let args = args
            .strip_suffix(&["--format", "json-compact"])
            .unwrap_or(&args);
        match args {
            ["gateway", "worktrees", "ensure", key] => {
                self.verbs.lock().unwrap().push(format!("worktree:{key}"));
                let repo = aidp_core::vault::repo_segment(&self.project);
                match aidp_gateway::worktrees::ensure(
                    &aidp_core::process_runner::OsProcessRunner,
                    &self.project,
                    &self.vault,
                    &repo,
                    key,
                    &[],
                ) {
                    Ok(ensured) => (
                        0,
                        json!({ "path": ensured.workspace.path, "base_sha": HEAD_SHA, "reused": ensured.reused }),
                    ),
                    Err(error) => (1, json!(format!("{error:?}"))),
                }
            }
            ["gateway", "worktrees", "remove", key] => {
                self.verbs
                    .lock()
                    .unwrap()
                    .push(format!("worktree-remove:{key}"));
                (0, json!({ "removed": true }))
            }
            ["flow", "git-branch", key] => {
                self.verbs.lock().unwrap().push(format!("branch:{key}"));
                (
                    0,
                    json!({ "branch": format!("feat/{key}"), "created": true }),
                )
            }
            ["flow", "git-checkout", key] => {
                self.verbs.lock().unwrap().push(format!("checkout:{key}"));
                if self.dirty.load(Ordering::SeqCst) {
                    let message = "blocked: dirty-checkout. Le checkout principal a des modifications locales sur `main` :
- `M Assets/Foo.cs`";
                    return (
                        0,
                        json!({ "branch": format!("feat/{key}"), "base": "main", "created": false,
                                "blocked": true, "blocked_reason": "dirty-checkout", "message": message }),
                    );
                }
                (
                    0,
                    json!({ "branch": format!("feat/{key}"), "base": "main", "created": true, "blocked": false }),
                )
            }
            ["flow", "git-restore", key] => {
                self.verbs.lock().unwrap().push(format!("restore:{key}"));
                (
                    0,
                    json!({ "restored": true, "from": format!("feat/{key}"), "base": "main" }),
                )
            }
            ["ticket", "label", key, state] => self.label(key, state),
            ["ticket", "comment", key, rest @ ..] => self.comment(key, rest),
            ["gh", "pr-ready", number] => {
                self.verbs
                    .lock()
                    .unwrap()
                    .push(format!("pr-ready:{number}"));
                (0, json!({ "changed": true }))
            }
            ["gh", "pr-merge", number, "--expected-head-sha", sha, "--report-conflict"] => {
                self.verbs
                    .lock()
                    .unwrap()
                    .push(format!("pr-merge:{number}:{sha}"));
                if self.merge_stalls.load(Ordering::SeqCst) {
                    return (
                        0,
                        json!({ "merged": false, "rebased": true, "branchDeleted": false }),
                    );
                }
                (0, json!({ "merged": true }))
            }
            other => (64, json!(format!("unexpected aidp verb {other:?}"))),
        }
    }

    fn verbs(&self) -> Vec<String> {
        self.verbs.lock().unwrap().clone()
    }

    fn labels_in_order(&self) -> Vec<String> {
        self.verbs()
            .iter()
            .filter_map(|verb| verb.strip_prefix("label:").map(str::to_string))
            .collect()
    }
}

fn flag_value<'a>(args: &[&'a str], flag: &str) -> Option<&'a str> {
    let index = args.iter().position(|argument| *argument == flag)?;
    args.get(index + 1).copied()
}

impl ProcessRunner for FakeAidp {
    fn output(&self, _: &ProcessSpec) -> std::io::Result<std::process::Output> {
        Err(std::io::Error::other("unexpected output call"))
    }

    fn spawn(&self, spec: &ProcessSpec) -> std::io::Result<Box<dyn SpawnedChild>> {
        assert_eq!(spec.program, "aidp", "only aidp verbs are spawned");
        let (code, value) = self.dispatch(&spec.args);
        let stdout = if code == 0 {
            value.to_string()
        } else {
            value.as_str().unwrap_or_default().to_string()
        };
        Ok(Box::new(Finished {
            stdout: stdout.into_bytes(),
            code,
        }))
    }
}

/// What the scripted provider answers, by pipeline.
struct Script {
    reviews: Vec<&'static str>,
    tests: Vec<bool>,
    /// The first session of this stage (`implement` or `test`) stays live this long.
    hold: Option<(&'static str, Duration)>,
}

impl Script {
    fn held(&self, call: &Call) -> Behaviour {
        match (call.nth, self.hold) {
            (1, Some((stage, hold))) if stage == call.label => Behaviour::ok().hold(hold),
            _ => Behaviour::ok(),
        }
    }

    fn behaviour(&self, call: &Call) -> Behaviour {
        let result = |value: Value| {
            std::fs::create_dir_all(&call.stage_dir).unwrap();
            std::fs::write(call.stage_dir.join("result.json"), value.to_string()).unwrap();
        };
        let pick = |count: usize| call.nth.min(count).saturating_sub(1);
        match call.label.as_str() {
            "implement" => {
                std::fs::create_dir_all(&call.stage_dir).unwrap();
                std::fs::write(
                    call.stage_dir.join("pr.json"),
                    json!({ "number": PR_NUMBER, "url": format!("https://github.com/{REPO}/pull/{PR_NUMBER}"),
                            "repo": REPO, "branch": "feat/x", "commit_sha": HEAD_SHA })
                    .to_string(),
                )
                .unwrap();
                result(json!({ "pr": PR_NUMBER, "branch": "feat/x", "summary": "implemented" }));
                self.held(call)
            }
            "review" => {
                let verdict = self.reviews[pick(self.reviews.len())];
                result(json!({ "verdict": verdict, "findings": [{ "id": call.nth }] }));
                Behaviour::ok()
            }
            "fix" => {
                result(json!({ "pr": PR_NUMBER, "branch": "feat/x", "summary": "fixed" }));
                Behaviour::ok()
            }
            "test" => {
                let passed = self.tests[pick(self.tests.len())];
                result(json!({ "passed": passed, "report": "tests" }));
                self.held(call)
            }
            other => panic!("unexpected stage {other}"),
        }
    }
}

struct World {
    engine: Arc<FlowEngine>,
    provider: Arc<ScriptedProvider>,
    cli: Arc<FakeAidp>,
    mock: MockHandle,
    project_id: String,
    vault: PathBuf,
    shutdown: Arc<aidp_gateway::shutdown::Shutdown>,
    background: aidp_gateway::flows::recovery::Background,
    _root: TempWorkspace,
    _env: EnvGuard,
}

fn world(tag: &str, script: Script, tickets: &[&str]) -> World {
    let root = TempWorkspace::new(tag);
    let vault = root.child("vault");
    let home = root.child("home");
    let workspace = root.child("workspace");
    let state_dir = root.child("state");
    for directory in [&vault, &home, &state_dir, &workspace] {
        std::fs::create_dir_all(directory).unwrap();
    }
    let origin = root.child("origin.git");
    GAME_FIXTURE.copy_with_origin_into(&workspace, &origin);
    let env = EnvGuard::with(&[
        ("AIDP_VAULT", Some(vault.to_str().unwrap())),
        ("HOME", Some(home.to_str().unwrap())),
        ("USERPROFILE", Some(home.to_str().unwrap())),
    ]);

    let mock_listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let mock = serve_on(mock_listener, &root.child("mocks")).unwrap();
    let mut issues = BTreeMap::new();
    let seeded: Vec<Value> = tickets
        .iter()
        .enumerate()
        .map(|(index, key)| {
            let number = index as u64 + 1;
            issues.insert((*key).to_string(), number);
            json!({ "number": number, "title": format!("issue {number}"), "body": "", "labels": [] })
        })
        .collect();
    ureq::post(&format!("{}/__seed_issues", mock.base_url()))
        .send_json(json!({ "repo": REPO, "issues": seeded }))
        .unwrap();
    let cli = Arc::new(FakeAidp {
        mock: mock.base_url(),
        project: workspace.clone(),
        vault: vault.clone(),
        dirty: AtomicBool::new(false),
        merge_stalls: AtomicBool::new(false),
        issues,
        verbs: Mutex::new(Vec::new()),
        snapshots: Mutex::new(Vec::new()),
    });

    let projects = Arc::new(Projects::load(&state_dir));
    let project_id = projects.add(workspace.to_str().unwrap()).id;
    let bus = Arc::new(Bus::new());
    let script = Arc::new(script);
    let provider = ScriptedProvider::new(None, move |call| script.behaviour(call));
    let scripted: Arc<dyn ProcessRunner> = Arc::new(ScriptedProcessRunner::new());
    let sessions =
        SessionManager::load_with_runner(&state_dir, bus.clone(), projects.clone(), scripted)
            .with_provider(PROVIDER_ID, provider.clone())
            .with_providers_config(ProvidersConfig {
                default: Some(PROVIDER_ID.to_string()),
                ..ProvidersConfig::default()
            });
    let skill_root = Path::new(env!("CARGO_MANIFEST_DIR")).join(SKILL_ROOT);
    let mut deps = NativeDeps::new(
        sessions.clone(),
        projects.clone(),
        bus.clone(),
        Arc::new(AlwaysAllow),
        Arc::new(LocalLock::default()),
        &state_dir,
        Some(skill_root),
    );
    deps.poll_interval = Duration::from_millis(10);
    let native = NativeRunManager::new(deps);
    let launcher = Arc::new(RunLauncher::new(sessions.clone(), native));
    let root_vault = vault.clone();
    let ports = Ports {
        sessions: Arc::new(
            ManagerSessions::new(sessions.clone(), projects.clone(), launcher)
                .with_vault_root(vault.clone()),
        ),
        budget: Arc::new(AlwaysAllow),
        lock: Arc::new(SessionLock::new(sessions)),
        bus,
        clock: Arc::new(SystemClock),
        ids: Arc::new(SystemIdGen),
        runner: cli.clone(),
        lock_wait: Duration::from_secs(5),
    };
    let engine = FlowEngine::new(
        vault,
        state_dir,
        registry_with_pr(),
        ports,
        FlowEngineConfig::default(),
    );
    let shutdown = Arc::new(aidp_gateway::shutdown::Shutdown::new());
    let background = aidp_gateway::flows::recovery::spawn_background(
        &engine,
        shutdown.clone(),
        Duration::from_secs(1),
    );
    World {
        engine,
        provider,
        cli,
        mock,
        project_id,
        vault: root_vault,
        shutdown,
        background,
        _root: root,
        _env: env,
    }
}

impl World {
    fn start(&self, key: &str) {
        self.start_flow(key, DEV_FLOW);
    }

    fn start_test(&self, key: &str) {
        self.start_flow(key, TEST_FLOW);
    }

    fn start_flow(&self, key: &str, flow: &str) {
        self.start_flow_at(key, flow, None);
    }

    fn start_flow_at(&self, key: &str, flow: &str, entry: Option<&str>) {
        let number = self.cli.issues[key];
        let ticket = json!({
            "key": key, "service": self.project_id, "summary": "game feature",
            "source": "github", "github": { "number": number },
        });
        let request = TicketFlowRequest {
            ticket,
            policy: Some(FlowExecutionPolicy {
                autonomous: true,
                provider: Some(PROVIDER_ID.to_string()),
                ..FlowExecutionPolicy::default()
            }),
            entry: entry.map(str::to_string),
            ..TicketFlowRequest::default()
        };
        self.engine
            .start_request_with_def(request, FlowDef::from_yaml(flow).unwrap())
            .expect("the flow starts");
    }

    fn state(&self, key: &str) -> FlowState {
        self.engine.get(key).expect("the flow exists")
    }

    fn wait(
        &self,
        what: &str,
        mut condition: impl FnMut(&FlowState) -> bool,
        key: &str,
    ) -> FlowState {
        let deadline = Instant::now() + SETTLE;
        loop {
            let state = self.state(key);
            if condition(&state) {
                return state;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {what}; flow {:?} at {:?}; steps {:?}; verbs {:?}; calls {:?}; runs {}",
                state.status,
                state.current,
                state.steps.iter().map(|(id, row)| format!("{id}={:?}/{:?}/{:?}", row.status, row.reason, row.detail)).collect::<Vec<_>>(),
                self.cli.verbs(),
                self.provider.labels(),
                dump_runs(&self.vault)
            );
            std::thread::sleep(POLL);
        }
    }

    fn wait_gate(&self, key: &str) -> FlowState {
        self.wait(
            "the merge gate",
            |state| step_is(state, "merge_approval", StepStatus::WaitingHuman),
            key,
        )
    }

    fn comments(&self, key: &str) -> Vec<String> {
        let number = self.cli.issues[key];
        let listed: Value = ureq::get(&self.cli.url(number, "/comments"))
            .call()
            .unwrap()
            .into_json()
            .unwrap();
        listed
            .as_array()
            .unwrap()
            .iter()
            .map(|comment| comment["body"].as_str().unwrap().to_string())
            .collect()
    }

    fn mock_labels(&self, key: &str) -> Vec<String> {
        self.cli.issue_labels(self.cli.issues[key])
    }

    fn finish(self) {
        self.shutdown.set();
        self.engine.shutdown().unwrap();
        self.background.join();
        self.mock.stop();
    }
}

fn status_of(state: &FlowState, step: &str) -> StepStatus {
    state
        .steps
        .get(step)
        .map_or(StepStatus::Pending, |row| row.status)
}

#[test]
fn dev_flow_stops_at_to_test_and_never_touches_the_main_checkout() {
    let world = world(
        "game-code-dev",
        Script {
            reviews: vec!["need-fix", "done"],
            tests: vec![true],
            hold: None,
        },
        &["GH-1"],
    );
    world.start("GH-1");
    let done = world.wait("the end", |state| state.status == FlowStatus::Done, "GH-1");

    assert_eq!(
        world.provider.labels(),
        ["implement", "review", "fix", "review"],
        "no Unity test in the dev lane"
    );
    assert_eq!(
        world.cli.labels_in_order(),
        [
            "aidp:in-progress",
            "aidp:review",
            "aidp:review",
            "aidp:to-test"
        ]
    );
    assert_eq!(world.mock_labels("GH-1"), ["aidp:to-test"]);
    let verbs = world.cli.verbs();
    let position = |wanted: &str| verbs.iter().position(|verb| verb == wanted).unwrap();
    assert_eq!(position("label:aidp:in-progress"), 0);
    assert!(position("worktree:GH-1") < position("branch:GH-1"));
    assert!(position("label:aidp:to-test") < position("worktree-remove:GH-1"));
    for forbidden in ["checkout:GH-1", "restore:GH-1"] {
        assert!(!verbs.contains(&forbidden.to_string()), "{verbs:?}");
    }
    assert!(
        !verbs
            .iter()
            .any(|verb| verb.starts_with("pr-ready") || verb.starts_with("pr-merge")),
        "nothing is made ready or merged by the dev lane: {verbs:?}"
    );
    let comments = world.comments("GH-1");
    assert_eq!(comments.len(), 2, "comments: {comments:?}");
    assert!(comments[0].starts_with("Revue unity-review : need-fix"));
    assert!(comments[1].starts_with("Revue unity-review : done"));
    assert_eq!(status_of(&done, "to_test"), StepStatus::Done);
    assert_eq!(status_of(&done, "clean"), StepStatus::Done);
    assert_eq!(status_of(&done, "review_exhausted"), StepStatus::Skipped);
    let (holder, _) = world
        .engine
        .executor()
        .resources()
        .snapshot(&format!("unity-editor:{}", world.project_id));
    assert!(holder.is_none(), "the dev lane never holds the editor");
    world.finish();
}

#[test]
fn test_flow_runs_the_test_loop_then_the_merge_in_the_main_checkout() {
    let world = world(
        "game-test-code-happy",
        Script {
            reviews: vec!["done"],
            tests: vec![false, true],
            hold: None,
        },
        &["GH-1"],
    );
    world.start_test("GH-1");
    let gate = world.wait_gate("GH-1");

    assert_eq!(gate.status, FlowStatus::WaitingHuman);
    assert_eq!(world.provider.labels(), ["test", "fix", "test"]);
    assert_eq!(world.mock_labels("GH-1"), ["aidp:awaiting-merge"]);
    assert!(
        !world
            .cli
            .verbs()
            .iter()
            .any(|verb| verb.starts_with("pr-merge")),
        "nothing merges before the approval"
    );

    world
        .engine
        .approve("GH-1", "merge_approval", "test")
        .unwrap();
    let done = world.wait("the end", |state| state.status == FlowStatus::Done, "GH-1");

    assert_eq!(
        world.cli.labels_in_order(),
        [
            "aidp:testing",
            "aidp:testing",
            "aidp:awaiting-merge",
            "aidp:done"
        ]
    );
    assert!(
        world
            .cli
            .snapshots
            .lock()
            .unwrap()
            .iter()
            .all(|labels| labels.len() == 1),
        "exactly one aidp:* label after each change"
    );
    assert_eq!(world.mock_labels("GH-1"), ["aidp:done"]);

    let verbs = world.cli.verbs();
    let position = |wanted: &str| verbs.iter().position(|verb| verb == wanted).unwrap();
    assert_eq!(
        position("checkout:GH-1"),
        0,
        "the checkout is the first verb"
    );
    assert!(position(&format!("pr-ready:{PR_NUMBER}")) < position("label:aidp:awaiting-merge"));
    assert!(position("restore:GH-1") < position(&format!("pr-merge:{PR_NUMBER}:{HEAD_SHA}")));
    assert!(position(&format!("pr-merge:{PR_NUMBER}:{HEAD_SHA}")) < position("label:aidp:done"));
    assert_eq!(
        verbs.iter().filter(|verb| *verb == "restore:GH-1").count(),
        1,
        "the daemon does not restore a second time"
    );
    assert!(
        !verbs.iter().any(|verb| verb.starts_with("worktree")),
        "no worktree in the test lane: {verbs:?}"
    );

    let comments = world.comments("GH-1");
    assert_eq!(comments.len(), 2, "comments: {comments:?}");
    for (comment, expected) in comments.iter().zip([
        "Test unity-test : passed=false",
        "Test unity-test : passed=true",
    ]) {
        assert!(comment.starts_with(expected), "{comment:?} vs {expected:?}");
        assert!(comment.contains(COST_LINE), "cost footer in {comment:?}");
    }
    let (holder, _) = world
        .engine
        .executor()
        .resources()
        .snapshot(&format!("unity-editor:{}", world.project_id));
    assert!(holder.is_none(), "the lock is released");
    assert_eq!(done.steps["test"].outputs["passed"], json!(true));
    assert_eq!(status_of(&done, "fix_test"), StepStatus::Skipped);
    assert_eq!(status_of(&done, "test_exhausted"), StepStatus::Skipped);
    assert_eq!(status_of(&done, "merge"), StepStatus::Done);
    assert_eq!(status_of(&done, "checkout_stop"), StepStatus::Skipped);
    world.finish();
}

#[test]
fn exhausted_review_loop_blocks_the_issue_and_escalates() {
    let world = world(
        "game-code-exhausted",
        Script {
            reviews: vec!["need-fix"],
            tests: vec![true],
            hold: None,
        },
        &["GH-1"],
    );
    world.start("GH-1");
    let state = world.wait(
        "the escalation",
        |state| state.status == FlowStatus::Escalated,
        "GH-1",
    );

    assert_eq!(status_of(&state, "review_exhausted"), StepStatus::Done);
    assert_eq!(status_of(&state, "review_escalate"), StepStatus::Escalated);
    assert_eq!(status_of(&state, "to_test"), StepStatus::Pending);
    assert_eq!(world.mock_labels("GH-1"), ["aidp:blocked"]);
    assert_eq!(
        world.cli.labels_in_order(),
        [
            "aidp:in-progress",
            "aidp:review",
            "aidp:review",
            "aidp:blocked"
        ]
    );
    assert_eq!(
        world.provider.labels(),
        ["implement", "review", "fix", "review", "fix"],
        "the last fix of an exhausted loop runs without a review after it"
    );
    assert_eq!(state.steps["review"].outputs["verdict"], json!("need-fix"));
    world.finish();
}

#[test]
fn exhausted_test_loop_blocks_the_issue_and_gives_the_checkout_back() {
    let world = world(
        "game-test-code-exhausted",
        Script {
            reviews: vec!["done"],
            tests: vec![false],
            hold: None,
        },
        &["GH-1"],
    );
    world.start_test("GH-1");
    let state = world.wait(
        "the escalation",
        |state| state.status == FlowStatus::Escalated,
        "GH-1",
    );

    assert_eq!(status_of(&state, "test_escalate"), StepStatus::Escalated);
    assert_eq!(world.provider.labels(), ["test", "fix", "test", "fix"]);
    assert_eq!(world.mock_labels("GH-1"), ["aidp:blocked"]);
    let deadline = Instant::now() + SETTLE;
    while !world.cli.verbs().contains(&"restore:GH-1".to_string()) {
        assert!(
            Instant::now() < deadline,
            "the daemon gives the checkout back"
        );
        std::thread::sleep(POLL);
    }
    let resource = format!("unity-editor:{}", world.project_id);
    let released = Instant::now() + SETTLE;
    while world
        .engine
        .executor()
        .resources()
        .snapshot(&resource)
        .0
        .is_some()
    {
        assert!(
            Instant::now() < released,
            "the lock is released after the escalation"
        );
        std::thread::sleep(POLL);
    }
    world.finish();
}

#[test]
fn second_test_flow_waits_for_the_unity_editor_then_runs() {
    let world = world(
        "game-test-code-lock",
        Script {
            reviews: vec!["done"],
            tests: vec![true],
            hold: Some(("test", Duration::from_millis(6000))),
        },
        &["GH-1", "GH-2"],
    );
    let resource = format!("unity-editor:{}", world.project_id);
    world.start_test("GH-1");
    world.wait(
        "the first test",
        |state| step_is(state, "test", StepStatus::Running) && world.provider.count("test") == 1,
        "GH-1",
    );
    world.start_test("GH-2");
    let waiting = world.wait(
        "the second flow to wait",
        |state| step_is(state, "checkout", StepStatus::WaitingExternal),
        "GH-2",
    );

    assert_eq!(
        waiting.steps["checkout"].detail.as_deref(),
        Some(UNITY_REASON)
    );
    let (holder, queue) = world.engine.executor().resources().snapshot(&resource);
    assert_eq!(holder.unwrap().flow_key, "GH-1");
    assert_eq!(queue, ["GH-2"]);
    let (_, held) = world.engine.lane_load();
    assert_eq!(
        held.values().sum::<usize>(),
        2,
        "both count in the test lane of the project"
    );
    assert_eq!(
        world.provider.count("test"),
        1,
        "the second test did not start"
    );

    world.wait_gate("GH-1");
    let second = world.wait_gate("GH-2");
    assert_eq!(world.provider.count("test"), 2);
    let first_end = world.state("GH-1").steps["restore"]
        .finished_at
        .clone()
        .unwrap();
    let second_start = second.steps["checkout"].started_at.clone().unwrap();
    assert!(first_end <= second_start, "{first_end} then {second_start}");
    let (holder, queue) = world.engine.executor().resources().snapshot(&resource);
    assert!(holder.is_none() && queue.is_empty(), "the lock is released");
    let (_, held) = world.engine.lane_load();
    assert!(
        held.is_empty(),
        "past the restore a flow waiting for the merge no longer holds the lane: {held:?}"
    );

    for key in ["GH-1", "GH-2"] {
        world.engine.approve(key, "merge_approval", "test").unwrap();
        world.wait("the end", |state| state.status == FlowStatus::Done, key);
        assert_eq!(world.mock_labels(key), ["aidp:done"]);
    }
    world.finish();
}

#[test]
fn dev_flows_run_in_parallel_and_a_test_flow_runs_beside_them() {
    let world = world(
        "game-code-parallel",
        Script {
            reviews: vec!["done"],
            tests: vec![true],
            hold: Some(("implement", Duration::from_millis(8000))),
        },
        &["GH-1", "GH-2", "GH-3"],
    );
    world.start("GH-1");
    world.wait(
        "the first implement",
        |state| step_is(state, "implement", StepStatus::Running),
        "GH-1",
    );
    world.start("GH-2");
    world.wait(
        "the second implement beside the first",
        |state| step_is(state, "implement", StepStatus::Running),
        "GH-2",
    );
    world.start_test("GH-3");
    world.wait(
        "the test flow beside the dev flows",
        |state| step_is(state, "test", StepStatus::Done),
        "GH-3",
    );
    let (dev, test) = world.engine.lane_load();
    assert_eq!(dev, 2);
    assert_eq!(test.values().sum::<usize>(), 1);
    for key in ["GH-1", "GH-2"] {
        world.wait("the dev end", |state| state.status == FlowStatus::Done, key);
        assert_eq!(world.mock_labels(key), ["aidp:to-test"]);
    }
    world.wait_gate("GH-3");
    world
        .engine
        .approve("GH-3", "merge_approval", "test")
        .unwrap();
    world.wait("the end", |state| state.status == FlowStatus::Done, "GH-3");
    world.finish();
}

#[test]
fn dirty_main_checkout_blocks_the_test_and_escalates() {
    let world = world(
        "game-test-code-dirty",
        Script {
            reviews: vec!["done"],
            tests: vec![true],
            hold: None,
        },
        &["GH-1"],
    );
    world.cli.dirty.store(true, Ordering::SeqCst);
    world.start_test("GH-1");
    let state = world.wait(
        "the escalation",
        |state| state.status == FlowStatus::Escalated,
        "GH-1",
    );

    assert_eq!(status_of(&state, "checkout"), StepStatus::Done);
    assert_eq!(status_of(&state, "checkout_stop"), StepStatus::Escalated);
    assert_eq!(status_of(&state, "test_loop"), StepStatus::Pending);
    assert_eq!(world.mock_labels("GH-1"), ["aidp:blocked"]);
    assert_eq!(world.cli.labels_in_order(), ["aidp:blocked"]);
    let comments = world.comments("GH-1");
    assert_eq!(comments.len(), 1, "comments: {comments:?}");
    assert!(
        comments[0].starts_with("blocked: dirty-checkout"),
        "{comments:?}"
    );
    assert!(comments[0].contains("Assets/Foo.cs"), "{comments:?}");
    assert!(world.provider.labels().is_empty(), "no pipeline ran");
    world.finish();
}

#[test]
fn merge_that_only_rebased_fails_the_step_and_never_posts_done() {
    let world = world(
        "game-test-code-merge-stalls",
        Script {
            reviews: vec!["done"],
            tests: vec![true],
            hold: None,
        },
        &["GH-1"],
    );
    world.cli.merge_stalls.store(true, Ordering::SeqCst);
    world.start_test("GH-1");
    world.wait_gate("GH-1");
    world
        .engine
        .approve("GH-1", "merge_approval", "test")
        .unwrap();
    let state = world.wait(
        "the failed merge",
        |state| step_is(state, "merge", StepStatus::Failed),
        "GH-1",
    );

    assert_ne!(state.status, FlowStatus::Done);
    assert!(
        !world
            .cli
            .labels_in_order()
            .contains(&"aidp:done".to_string()),
        "no done label: {:?}",
        world.cli.labels_in_order()
    );
    assert_ne!(world.mock_labels("GH-1"), ["aidp:done"]);
    world.finish();
}

fn step_is(state: &FlowState, step: &str, status: StepStatus) -> bool {
    state
        .steps
        .get(step)
        .is_some_and(|row| row.status == status)
}

/// Every `run-state.json` under `dir`, for the failure message of a timed out wait.
fn dump_runs(dir: &Path) -> String {
    let mut found = String::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            found.push_str(&dump_runs(&path));
        } else if path
            .file_name()
            .is_some_and(|name| name == "run-state.json")
        {
            let text = std::fs::read_to_string(&path).unwrap_or_default();
            found.push_str(&format!(
                "\n{}: {}",
                path.display(),
                &text
                    .replace(
                        "
", ""
                    )
                    .replace("  ", "")
            ));
        }
    }
    found
}

#[test]
fn merge_entry_skips_the_checkout_and_the_unity_test_then_merges() {
    let world = world(
        "game-test-code-forced-merge",
        Script {
            reviews: vec!["done"],
            tests: vec![false],
            hold: None,
        },
        &["GH-1"],
    );
    world.start_flow_at("GH-1", TEST_FLOW, Some(ENTRY_MERGE));
    let gate = world.wait_gate("GH-1");

    assert!(world.provider.labels().is_empty(), "no pipeline runs");
    assert_eq!(world.mock_labels("GH-1"), ["aidp:awaiting-merge"]);
    assert_eq!(status_of(&gate, "checkout"), StepStatus::Skipped);
    assert_eq!(status_of(&gate, "test_loop"), StepStatus::Skipped);
    assert_eq!(status_of(&gate, "test_exhausted"), StepStatus::Skipped);
    assert_eq!(gate.entry.as_deref(), Some(ENTRY_MERGE));

    world
        .engine
        .approve("GH-1", "merge_approval", "test")
        .unwrap();
    let done = world.wait("the end", |state| state.status == FlowStatus::Done, "GH-1");

    let verbs = world.cli.verbs();
    assert!(!verbs.contains(&"checkout:GH-1".to_string()), "{verbs:?}");
    assert!(
        verbs.contains(&format!("pr-ready:{PR_NUMBER}")),
        "{verbs:?}"
    );
    assert!(
        verbs.contains(&format!("pr-merge:{PR_NUMBER}:{HEAD_SHA}")),
        "{verbs:?}"
    );
    assert_eq!(status_of(&done, "merge"), StepStatus::Done);
    assert_eq!(world.mock_labels("GH-1"), ["aidp:done"]);
    world.finish();
}

#[test]
fn review_entry_prepares_the_worktree_then_reviews_without_implementing() {
    let world = world(
        "game-code-forced-review",
        Script {
            reviews: vec!["done"],
            tests: vec![true],
            hold: None,
        },
        &["GH-1"],
    );
    world.start_flow_at("GH-1", DEV_FLOW, Some(ENTRY_REVIEW));
    let done = world.wait("the end", |state| state.status == FlowStatus::Done, "GH-1");

    assert_eq!(world.provider.labels(), ["review"], "implement is skipped");
    let verbs = world.cli.verbs();
    assert!(verbs.contains(&"worktree:GH-1".to_string()), "{verbs:?}");
    assert!(verbs.contains(&"branch:GH-1".to_string()), "{verbs:?}");
    assert!(
        !verbs.contains(&"label:aidp:in-progress".to_string()),
        "{verbs:?}"
    );
    assert_eq!(status_of(&done, "implement"), StepStatus::Skipped);
    assert_eq!(status_of(&done, "to_test"), StepStatus::Done);
    assert_eq!(world.mock_labels("GH-1"), ["aidp:to-test"]);
    world.finish();
}
