use aidp_agents::{AgentEvent, McpConfiguration, PermissionPolicy, ProvidersConfig};
use aidp_core::process_runner::ScriptedProcessRunner;
use aidp_gateway::bus::Bus;
use aidp_gateway::models::constants::mcp_configuration::UNKNOWN_REFERENCE;
use aidp_gateway::models::enums::SessionStatus;
use aidp_gateway::projects::Projects;
use aidp_gateway::sessions::{Launch, SessionManager, StartRequest};
use aidp_testkit::fake_provider::FakeProvider;
use aidp_testkit::fake_provider_script::FakeScript;
use aidp_testkit::{fixtures, EnvGuard, TempWorkspace};
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

const STREAM: &str = include_str!("fixtures/fake_provider/stream.jsonl");
const FAKE_PROVIDER_ID: &str = "fake";
const MCP_CONFIGURATION_ID: &str = "session-runtime";
const MCP_PRIVATE_KEY: &str = "PRIVATE_MCP_VALUE";
const MCP_PRIVATE_VALUE: &str = "session-runtime-private-value";
const STATE_DIRECTORY: &str = "state";
const PROJECT_DIRECTORY: &str = "project";
const HOME_DIRECTORY: &str = "home";
const SKILL_ROOT_ENV: &str = "AIDP_SKILL_ROOT";
const VAULT_ENV: &str = "AIDP_VAULT";
const HOME_ENV: &str = "HOME";
const USERPROFILE_ENV: &str = "USERPROFILE";
const VAULT_DISABLED: &str = "off";
const PROMPT_ENGINE: &str = "prompt";
const PROMPT: &str = "hello from the provider test";
const WAIT: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(20);
const HEADLESS_RESUME_TAG: &str = "session_headless_resume";
const DONT_ASK_MODE: &str = "dontAsk";

fn start_request(project_id: &str) -> StartRequest {
    StartRequest {
        project_id: project_id.to_string(),
        engine: Some(PROMPT_ENGINE.to_string()),
        prompt: Some(PROMPT.to_string()),
        ..Default::default()
    }
}

fn wait_for_completion(manager: &SessionManager, session_id: &str) {
    let deadline = Instant::now() + WAIT;
    while !manager
        .get(session_id)
        .is_some_and(|record| record.status == SessionStatus::Done)
    {
        assert!(
            Instant::now() < deadline,
            "timed out waiting for completion"
        );
        std::thread::sleep(POLL);
    }
}

/// A session manager over `state_directory` whose default provider is the fake `provider`.
fn fake_manager(
    state_directory: &std::path::Path,
    provider: &Arc<FakeProvider>,
) -> Arc<SessionManager> {
    SessionManager::load_with_runner(
        state_directory,
        Arc::new(Bus::new()),
        Arc::new(Projects::load(state_directory)),
        Arc::new(ScriptedProcessRunner::new()),
    )
    .with_provider(FAKE_PROVIDER_ID, provider.clone())
    .with_providers_config(ProvidersConfig {
        default: Some(FAKE_PROVIDER_ID.to_string()),
        ..Default::default()
    })
}

/// A resume of a headless session stays headless; both records reload headless.
#[test]
fn resume_of_a_headless_session_stays_headless_after_restart() {
    let workspace = TempWorkspace::new(HEADLESS_RESUME_TAG);
    let state_directory = workspace.child(STATE_DIRECTORY);
    let project_directory = workspace.child(PROJECT_DIRECTORY);
    let home = workspace.child(HOME_DIRECTORY);
    for directory in [&state_directory, &project_directory, &home] {
        std::fs::create_dir_all(directory).unwrap();
    }
    let skill = fixtures::skill_root();
    let _environment = EnvGuard::with(&[
        (SKILL_ROOT_ENV, Some(skill.to_str().unwrap())),
        (VAULT_ENV, Some(VAULT_DISABLED)),
        (HOME_ENV, Some(home.to_str().unwrap())),
        (USERPROFILE_ENV, Some(home.to_str().unwrap())),
    ]);
    let mut script = FakeScript::new();
    for line in STREAM.lines() {
        script = script.emit(AgentEvent::Raw(serde_json::from_str(line).unwrap()));
    }
    let provider = Arc::new(FakeProvider::new(script.exit_code(0)));
    let project = Projects::load(&state_directory).add(project_directory.to_str().unwrap());
    let manager = fake_manager(&state_directory, &provider);
    let mut request = start_request(&project.id);
    request.headless = true;
    let Launch::Started(parent) = manager.start(request).unwrap() else {
        panic!("session must start");
    };
    assert!(parent.headless);
    assert_eq!(parent.permission_mode, DONT_ASK_MODE);
    wait_for_completion(&manager, &parent.id);

    let mut resume = start_request(&project.id);
    resume.resume_from = Some(parent.id.clone());
    let Launch::Started(child) = manager.start(resume).unwrap() else {
        panic!("resume must start");
    };
    assert!(child.headless);
    assert_eq!(child.permission_mode, DONT_ASK_MODE);
    wait_for_completion(&manager, &child.id);
    for launched in provider.launched() {
        assert_eq!(launched.permission_policy, PermissionPolicy::Headless);
    }
    let restarted = fake_manager(&state_directory, &provider);
    for session in [&parent.id, &child.id] {
        assert!(restarted.get(session).unwrap().headless);
    }
}

#[test]
fn resume_inherits_runtime_mcp_reference_and_refuses_it_after_runtime_restart() {
    let workspace = TempWorkspace::new("session_runtime_mcp_resume");
    let state_directory = workspace.child(STATE_DIRECTORY);
    let project_directory = workspace.child(PROJECT_DIRECTORY);
    let home = workspace.child(HOME_DIRECTORY);
    for directory in [&state_directory, &project_directory, &home] {
        std::fs::create_dir_all(directory).unwrap();
    }
    let skill = fixtures::skill_root();
    let _environment = EnvGuard::with(&[
        (SKILL_ROOT_ENV, Some(skill.to_str().unwrap())),
        (VAULT_ENV, Some(VAULT_DISABLED)),
        (HOME_ENV, Some(home.to_str().unwrap())),
        (USERPROFILE_ENV, Some(home.to_str().unwrap())),
    ]);
    let mut script = FakeScript::new();
    for line in STREAM.lines() {
        script = script.emit(AgentEvent::Raw(serde_json::from_str(line).unwrap()));
    }
    let provider = Arc::new(FakeProvider::new(script.exit_code(0)));
    let bus = Arc::new(Bus::new());
    let projects = Arc::new(Projects::load(&state_directory));
    let project = projects.add(project_directory.to_str().unwrap());
    let manager = SessionManager::load_with_runner(
        &state_directory,
        bus.clone(),
        projects,
        Arc::new(ScriptedProcessRunner::new()),
    )
    .with_provider(FAKE_PROVIDER_ID, provider.clone())
    .with_providers_config(ProvidersConfig {
        default: Some(FAKE_PROVIDER_ID.to_string()),
        ..Default::default()
    });
    manager
        .mcp_configurations()
        .upsert(
            MCP_CONFIGURATION_ID.to_string(),
            McpConfiguration {
                servers: Vec::new(),
                environment: BTreeMap::from([(
                    MCP_PRIVATE_KEY.to_string(),
                    MCP_PRIVATE_VALUE.to_string(),
                )]),
            },
        )
        .unwrap();
    let mut request = start_request(&project.id);
    request.mcp_config_id = Some(MCP_CONFIGURATION_ID.to_string());
    let Launch::Started(parent) = manager.start(request).unwrap() else {
        panic!("session must start");
    };
    wait_for_completion(&manager, &parent.id);
    let mut resume = start_request(&project.id);
    resume.resume_from = Some(parent.id.clone());
    let Launch::Started(child) = manager.start(resume.clone()).unwrap() else {
        panic!("resume must start");
    };
    assert_eq!(child.mcp_config_id.as_deref(), Some(MCP_CONFIGURATION_ID));
    assert_eq!(
        provider
            .launched()
            .last()
            .unwrap()
            .mcp_configuration
            .as_ref()
            .unwrap()
            .environment[MCP_PRIVATE_KEY],
        MCP_PRIVATE_VALUE
    );
    wait_for_completion(&manager, &child.id);
    let restarted = SessionManager::load_with_runner(
        &state_directory,
        bus,
        Arc::new(Projects::load(&state_directory)),
        Arc::new(ScriptedProcessRunner::new()),
    );
    let error = match restarted.start(resume) {
        Err(error) => error,
        Ok(_) => panic!("missing MCP configuration must refuse resume"),
    };
    assert_eq!(error.to_string(), UNKNOWN_REFERENCE);
    assert_eq!(provider.launched().len(), 2);
    assert_eq!(restarted.running_count(), 0);
}
