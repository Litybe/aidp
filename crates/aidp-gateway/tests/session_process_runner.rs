//! LOC-32 scenario 4: `SessionManager` driven entirely through an injected
//! [`ProcessRunner`] — no `claude` binary on `PATH`, no real child process.
//! A [`ScriptedProcessRunner`] serves a recorded stream-json session as the
//! spawned child's stdout; `SessionManager::load_with_runner` wires it in.

use std::sync::Arc;

use aidp_core::process_runner::{ProcessRunner, ScriptedProcessRunner};
use aidp_gateway::bus::Bus;
use aidp_gateway::models::enums::SessionStatus;
use aidp_gateway::projects::Projects;
use aidp_gateway::sessions::{Launch, SessionManager, StartRequest};
use aidp_testkit::TempWorkspace;

const FIXTURE: &str = include_str!("fixtures/claude_session.jsonl");

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
        prompt: Some("hello from the scripted test".to_string()),
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

#[test]
fn start_replays_a_scripted_claude_session_with_path_emptied() {
    let path_guard = aidp_testkit::EnvGuard::set("PATH", "");
    let ws = TempWorkspace::new("session_process_runner");
    let state_dir = ws.child("state");
    std::fs::create_dir_all(&state_dir).unwrap();
    let project_dir = ws.child("project");
    std::fs::create_dir_all(&project_dir).unwrap();

    let bus = Arc::new(Bus::new());
    let projects = Arc::new(Projects::load(&state_dir));
    let project = projects.add(project_dir.to_str().unwrap());

    let runner: Arc<dyn ProcessRunner> =
        Arc::new(ScriptedProcessRunner::new().with_spawn_stdout(FIXTURE.as_bytes().to_vec()));
    let manager = SessionManager::load_with_runner(&state_dir, bus.clone(), projects, runner);

    let launch = manager
        .start(start_request(project.id.clone()))
        .expect("start must succeed with no claude on PATH");
    let session_id = match launch {
        Launch::Started(rec) => rec.id.clone(),
        Launch::Queued(_) => panic!("expected an immediate launch, got a queued one"),
    };

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        if let Some(rec) = manager.get(&session_id) {
            if rec.status != SessionStatus::Running {
                assert_eq!(rec.status, SessionStatus::Done);
                break;
            }
        }
        assert!(
            std::time::Instant::now() < deadline,
            "session never left the running state"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }

    let events_deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let types: Vec<String> = loop {
        let events = bus.recent(1000, 0);
        let types: Vec<String> = events
            .iter()
            .filter_map(|e| e["type"].as_str().map(str::to_string))
            .collect();
        if types.iter().any(|t| t == "session.ended")
            || std::time::Instant::now() >= events_deadline
        {
            break types;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    assert!(
        types.iter().any(|t| t == "session.started"),
        "missing session.started in {types:?}"
    );
    assert!(
        types.iter().any(|t| t == "session.turn_end"),
        "missing session.turn_end in {types:?}"
    );
    assert!(
        types.iter().any(|t| t == "session.ended"),
        "missing session.ended in {types:?}"
    );

    drop(path_guard);
}
