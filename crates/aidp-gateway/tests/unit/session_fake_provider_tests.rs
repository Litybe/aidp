use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::bus::Bus;
use crate::models::enums::SessionStatus;
use crate::projects::Projects;
use crate::sessions::{Launch, SessionManager, StartRequest};
use aidp_agents::{AgentEvent, ProviderEntry, ProvidersConfig, TokenUsage};
use aidp_core::process_runner::{ProcessRunner, ScriptedProcessRunner};
use aidp_testkit::fake_provider::FakeProvider;
use aidp_testkit::fake_provider_script::FakeScript;
use aidp_testkit::TempWorkspace;
use serde_json::Value;

const STREAM: &str = include_str!("../fixtures/fake_provider/stream.jsonl");
const FAKE_PROVIDER_ID: &str = "fake";
const UNKNOWN_PROVIDER_ID: &str = "no-such-provider";
const TIER: &str = "opus";
const CONFIGURED_MODEL: &str = "fake-model-9";
const REPLY_FILE: &str = "reply.md";
const SESSION_PLACEHOLDER: &str = "<SESSION>";
const PROJECT_PLACEHOLDER: &str = "<PROJECT>";
const VOLATILE_KEYS: &[&str] = &["id", "ts", "projectId"];
const WAIT: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(20);
const BACKSLASH: char = '\\';
const ENDED_EVENT: &str = "session.ended";

fn start_request(project_id: String) -> StartRequest {
    StartRequest {
        project_id,
        shared_writer_group: None,
        allowed_tools: Vec::new(),
        add_dirs: Vec::new(),
        engine: Some("prompt".to_string()),
        template_id: None,
        target: None,
        inputs: None,
        prompt: Some("hello from the provider test".to_string()),
        permission_mode: None,
        model: None,
        effort: None,
        max_budget_usd: None,
        origin: None,
        automation_id: None,
        automation_run_id: None,
        resume_from: None,
        auto_close: None,
        priority: None,
        context_from: None,
        auto_approve_gates: None,
        inputs_file: None,
        worktree: None,
        workspace: None,
        title: None,
        external: None,
        env: Default::default(),
        provider: None,
        ..StartRequest::default()
    }
}

fn fake_script() -> FakeScript {
    let mut script = FakeScript::new();
    for line in STREAM.lines() {
        let raw: Value = serde_json::from_str(line).expect("fixture line is json");
        script = script.emit(AgentEvent::Raw(raw));
    }
    script
        .text("Working on it.")
        .usage(TokenUsage {
            input: 10,
            output: 5,
            cache_creation: 0,
            cache_read: 0,
        })
        .write_file(REPLY_FILE, "Done.")
        .exit_code(0)
}

struct Harness {
    bus: Arc<Bus>,
    manager: Arc<SessionManager>,
    project_id: String,
    project_dir: String,
    _ws: TempWorkspace,
}

fn harness(name: &str, build: impl FnOnce(Arc<SessionManager>) -> Arc<SessionManager>) -> Harness {
    let ws = TempWorkspace::new(name);
    let state_dir = ws.child("state");
    std::fs::create_dir_all(&state_dir).unwrap();
    let project_dir = ws.child("project");
    std::fs::create_dir_all(&project_dir).unwrap();
    let bus = Arc::new(Bus::new());
    let projects = Arc::new(Projects::load(&state_dir));
    let project = projects.add(project_dir.to_str().unwrap());
    let runner: Arc<dyn ProcessRunner> =
        Arc::new(ScriptedProcessRunner::new().with_spawn_stdout(STREAM.as_bytes().to_vec()));
    let manager = build(SessionManager::load_with_runner(
        &state_dir,
        bus.clone(),
        projects,
        runner,
    ));
    Harness {
        bus,
        manager,
        project_id: project.id.clone(),
        project_dir: project_dir.to_str().unwrap().to_string(),
        _ws: ws,
    }
}

fn wait_until(mut done: impl FnMut() -> bool, what: &str) {
    let deadline = Instant::now() + WAIT;
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(POLL);
    }
}

fn scrub(value: &Value, session_id: &str, project_dir: &str) -> Value {
    match value {
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(key, _)| !VOLATILE_KEYS.contains(&key.as_str()))
                .map(|(key, inner)| (key.clone(), scrub(inner, session_id, project_dir)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|inner| scrub(inner, session_id, project_dir))
                .collect(),
        ),
        Value::String(text) => Value::String(
            text.replace(BACKSLASH, "/")
                .replace(session_id, SESSION_PLACEHOLDER)
                .replace(project_dir, PROJECT_PLACEHOLDER),
        ),
        other => other.clone(),
    }
}

fn run_session(harness: &Harness) -> (String, Vec<Value>) {
    let launch = harness
        .manager
        .start(start_request(harness.project_id.clone()))
        .expect("start must succeed");
    let session_id = match launch {
        Launch::Started(record) => record.id.clone(),
        Launch::Queued(_) => panic!("expected an immediate launch"),
    };
    wait_until(
        || {
            harness
                .manager
                .get(&session_id)
                .is_some_and(|record| record.status != SessionStatus::Running)
        },
        "session to leave running",
    );
    let status = harness.manager.get(&session_id).unwrap().status;
    assert_eq!(status, SessionStatus::Done);
    wait_until(
        || {
            harness
                .bus
                .recent(1000, 0)
                .iter()
                .any(|event| event["type"] == ENDED_EVENT)
        },
        "session.ended event",
    );
    let project_dir = harness.project_dir.replace(BACKSLASH, "/");
    let events = harness
        .bus
        .recent(1000, 0)
        .iter()
        .map(|event| scrub(event, &session_id, &project_dir))
        .collect();
    (session_id, events)
}

#[test]
fn fake_provider_session_matches_the_builtin_provider_events() {
    let builtin = harness("session_fake_provider_builtin", |manager| manager);
    let (_, builtin_events) = run_session(&builtin);

    let provider = Arc::new(FakeProvider::new(fake_script()));
    let fake = harness("session_fake_provider_fake", |manager| {
        let config = ProvidersConfig {
            default: Some(FAKE_PROVIDER_ID.to_string()),
            ..ProvidersConfig::default()
        };
        manager
            .with_provider(FAKE_PROVIDER_ID, provider.clone())
            .with_providers_config(config)
    });
    let (_, fake_events) = run_session(&fake);

    assert_eq!(provider.launched().len(), 1);
    assert_eq!(fake_events, builtin_events);
    let reply = Path::new(&provider.launched()[0].work_dir).join(REPLY_FILE);
    assert_eq!(std::fs::read_to_string(reply).unwrap(), "Done.");
}

#[test]
fn unknown_provider_fails_launch_naming_the_id() {
    let config = ProvidersConfig {
        default: Some(UNKNOWN_PROVIDER_ID.to_string()),
        ..ProvidersConfig::default()
    };
    let unknown = harness("session_fake_provider_unknown", |manager| {
        manager.with_providers_config(config)
    });
    let error = match unknown
        .manager
        .start(start_request(unknown.project_id.clone()))
    {
        Ok(_) => panic!("launch with an unknown provider must fail"),
        Err(error) => error,
    };
    assert!(
        error.to_string().contains(UNKNOWN_PROVIDER_ID),
        "error was: {error}"
    );
}

/// Config whose `fake` entry maps the `opus` tier to a concrete model.
fn tiered_config(default: Option<&str>) -> ProvidersConfig {
    let mut entry = ProviderEntry::default();
    entry
        .models
        .insert(TIER.to_string(), CONFIGURED_MODEL.to_string());
    let mut config = ProvidersConfig {
        default: default.map(str::to_string),
        ..ProvidersConfig::default()
    };
    config.entries.insert(FAKE_PROVIDER_ID.to_string(), entry);
    config
}

/// A configured tier resolves to the configured model of the default provider.
#[test]
fn configured_tier_maps_to_the_configured_model() {
    let provider = Arc::new(FakeProvider::new(fake_script()));
    let tiered = harness("session_provider_tier", |manager| {
        manager
            .with_provider(FAKE_PROVIDER_ID, provider.clone())
            .with_providers_config(tiered_config(Some(FAKE_PROVIDER_ID)))
    });

    let (resolved, model) = tiered
        .manager
        .resolve_launch_provider(Some(TIER), None)
        .expect("tier resolves");

    assert_eq!(resolved.id(), FAKE_PROVIDER_ID);
    assert_eq!(model.as_deref(), Some(CONFIGURED_MODEL));
}

/// The launch override selects the provider although the config default is another one.
#[test]
fn launch_override_selects_the_provider() {
    let provider = Arc::new(FakeProvider::new(fake_script()));
    let overridden = harness("session_provider_override", |manager| {
        manager
            .with_provider(FAKE_PROVIDER_ID, provider.clone())
            .with_providers_config(tiered_config(None))
    });

    let (resolved, model) = overridden
        .manager
        .resolve_launch_provider(Some(TIER), Some(FAKE_PROVIDER_ID))
        .expect("override resolves");

    assert_eq!(resolved.id(), FAKE_PROVIDER_ID);
    assert_eq!(model.as_deref(), Some(CONFIGURED_MODEL));
}
