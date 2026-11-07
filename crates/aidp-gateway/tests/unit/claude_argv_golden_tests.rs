//! The `claude` command line of a launched session, recorded and compared with a fixture.

use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crate::bus::Bus;
use crate::projects::Projects;
use crate::sessions::{Launch, SessionManager, StartRequest};
use aidp_core::process_runner::{ProcessRunner, ProcessSpec, SpawnedChild};
use aidp_testkit::{EnvGuard, TempWorkspace};
use serde::{Deserialize, Serialize};

const UPDATE_ENV: &str = "UPDATE_GOLDEN";
const PIPELINE_FIXTURE: &str = "tests/fixtures/claude_argv/pipeline_session.json";
const FREE_FIXTURE: &str = "tests/fixtures/claude_argv/free_session.json";
const NATIVE_STAGE_FIXTURE: &str = "tests/fixtures/claude_argv/native_stage_session.json";
const HEADLESS_STAGE_FIXTURE: &str = "tests/fixtures/claude_argv/headless_stage_session.json";
const AUTO_MODE: &str = "auto";
const RUN_DIR: &str = "run-dir";
const VAULT_DIR: &str = "vault";
const DEFAULT_MODE: &str = "default";
const DENIED_SHELL_COMMAND: &str = "git push";
const AIDP_BIN_KEY: &str = "AIDP_BIN";
const PATH_KEY: &str = "PATH";
const AIDP_BIN_PLACEHOLDER: &str = "<aidp-bin>";
const PATH_PLACEHOLDER: &str = "<path>";
const PROJECT_DIR_NAME: &str = "project";
const PROJECT_PLACEHOLDER: &str = "<project>";
const ENGINE_V7: &str = "v7";
const ENGINE_PROMPT: &str = "prompt";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct RecordedLaunch {
    program: String,
    args: Vec<String>,
    cwd: String,
    piped_stdin: bool,
    piped_stdout: bool,
    piped_stderr: bool,
    envs: BTreeMap<String, String>,
}

struct EmptyChild;

impl SpawnedChild for EmptyChild {
    fn take_stdout(&mut self) -> Option<Box<dyn Read + Send>> {
        Some(Box::new(Cursor::new(Vec::new())))
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
        Ok(Some(0))
    }
    fn kill(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[derive(Default)]
struct RecordingRunner {
    specs: Mutex<Vec<ProcessSpec>>,
}

impl ProcessRunner for RecordingRunner {
    fn output(&self, _spec: &ProcessSpec) -> std::io::Result<std::process::Output> {
        Err(std::io::Error::other("output is not used by sessions"))
    }
    fn spawn(&self, spec: &ProcessSpec) -> std::io::Result<Box<dyn SpawnedChild>> {
        self.specs.lock().unwrap().push(spec.clone());
        Ok(Box::new(EmptyChild))
    }
}

fn normalize(spec: &ProcessSpec) -> RecordedLaunch {
    let cwd = spec.cwd.clone().unwrap_or_default();
    let cwd = if cwd.ends_with(PROJECT_DIR_NAME) {
        PROJECT_PLACEHOLDER.to_string()
    } else {
        cwd.to_string_lossy().to_string()
    };
    let envs = spec
        .envs
        .iter()
        .map(|(key, value)| {
            let value = match key.as_str() {
                AIDP_BIN_KEY => AIDP_BIN_PLACEHOLDER.to_string(),
                PATH_KEY => PATH_PLACEHOLDER.to_string(),
                _ => value.clone(),
            };
            (key.clone(), value)
        })
        .collect();
    RecordedLaunch {
        program: spec.program.clone(),
        args: spec.args.clone(),
        cwd,
        piped_stdin: spec.piped_stdin,
        piped_stdout: spec.piped_stdout,
        piped_stderr: spec.piped_stderr,
        envs,
    }
}

fn record_launch(tag: &str, request: impl FnOnce(String) -> StartRequest) -> RecordedLaunch {
    let current_exe = std::env::current_exe().unwrap();
    let _env = EnvGuard::set(AIDP_BIN_KEY, current_exe.to_str().unwrap());
    let ws = TempWorkspace::new(tag);
    let state_dir = ws.child("state");
    std::fs::create_dir_all(&state_dir).unwrap();
    let project_dir = ws.child(PROJECT_DIR_NAME);
    std::fs::create_dir_all(&project_dir).unwrap();
    let bus = Arc::new(Bus::new());
    let projects = Arc::new(Projects::load(&state_dir));
    let project = projects.add(project_dir.to_str().unwrap());
    let runner = Arc::new(RecordingRunner::default());
    let manager = SessionManager::load_with_runner(&state_dir, bus, projects, runner.clone());
    match manager.start(request(project.id.clone())).unwrap() {
        Launch::Started(_) => {}
        Launch::Queued(_) => panic!("expected an immediate launch"),
    }
    let specs = runner.specs.lock().unwrap();
    assert_eq!(specs.len(), 1);
    normalize(&specs[0])
}

fn assert_matches_fixture(recorded: &RecordedLaunch, fixture: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(fixture);
    if std::env::var(UPDATE_ENV).is_ok() {
        let text = serde_json::to_string_pretty(recorded).unwrap() + "\n";
        std::fs::write(&path, text).unwrap();
    }
    let expected: RecordedLaunch =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(&expected, recorded);
}

fn base_request(project_id: String) -> StartRequest {
    StartRequest {
        project_id,
        ..StartRequest::default()
    }
}

#[test]
fn it_pipeline_session_command_line_is_stable() {
    let recorded = record_launch("claude_argv_pipeline", |project_id| StartRequest {
        engine: Some(ENGINE_V7.to_string()),
        template_id: Some("dev".to_string()),
        target: Some("PIXEL-1".to_string()),
        permission_mode: Some("acceptEdits".to_string()),
        model: Some("opus".to_string()),
        effort: Some("high".to_string()),
        max_budget_usd: Some(5.0),
        auto_approve_gates: Some(vec!["plan".to_string()]),
        ..base_request(project_id)
    });
    assert_matches_fixture(&recorded, PIPELINE_FIXTURE);
}

#[test]
fn it_free_session_command_line_is_stable() {
    let recorded = record_launch("claude_argv_free", |project_id| StartRequest {
        engine: Some(ENGINE_PROMPT.to_string()),
        prompt: Some("hello".to_string()),
        ..base_request(project_id)
    });
    assert_matches_fixture(&recorded, FREE_FIXTURE);
}

#[test]
fn it_native_stage_session_command_line_is_stable() {
    let recorded = record_launch("claude_argv_native_stage", |project_id| StartRequest {
        engine: Some(ENGINE_PROMPT.to_string()),
        prompt: Some("stage prompt".to_string()),
        permission_mode: Some(AUTO_MODE.to_string()),
        add_dirs: vec![PathBuf::from(RUN_DIR)],
        ..base_request(project_id)
    });
    assert_matches_fixture(&recorded, NATIVE_STAGE_FIXTURE);
}

#[test]
fn it_headless_stage_session_command_line_is_stable() {
    let recorded = record_launch("claude_argv_headless_stage", |project_id| StartRequest {
        engine: Some(ENGINE_PROMPT.to_string()),
        prompt: Some("stage prompt".to_string()),
        permission_mode: Some(DEFAULT_MODE.to_string()),
        strict_mcp_config: Some(true),
        denied_shell_commands: vec![DENIED_SHELL_COMMAND.to_string()],
        add_dirs: vec![PathBuf::from(RUN_DIR), PathBuf::from(VAULT_DIR)],
        headless: true,
        ..base_request(project_id)
    });
    assert_matches_fixture(&recorded, HEADLESS_STAGE_FIXTURE);
}
