use super::*;
use crate::models::enums::agent_executor::AgentExecutor;
use aidp_core::clock::SystemClock;
use std::sync::atomic::{AtomicBool, Ordering};

const ANY_KEY: &str = "KEY-1";
const REJECTED_RATE_STATUS: &str = "rejected";
const VAULT_DIRECTORY: &str = "vault";

/// A registry built with a PR status cache asks its locator, which the null source never does.
#[test]
fn registry_with_pr_status_reads_through_the_locator() {
    let consulted = Arc::new(AtomicBool::new(false));
    let flag = consulted.clone();
    let locate: TicketLocator = Arc::new(move |_key| {
        flag.store(true, Ordering::SeqCst);
        None
    });
    let cache = Arc::new(PrStatusCache::new(Box::new(SystemClock)));
    let registry = Registry::with_pr_status(cache, locate);
    assert!(registry.pr_status_source().status(ANY_KEY).is_none());
    assert!(consulted.load(Ordering::SeqCst));
}

/// A plain builtin registry never consults any locator.
#[test]
fn builtin_registry_hands_probes_the_null_source() {
    assert!(Registry::with_builtins()
        .pr_status_source()
        .status(ANY_KEY)
        .is_none());
}

#[test]
fn claude_is_blocked_by_the_subscription_window() {
    let directory = tempfile::tempdir().unwrap();
    let state = directory.path();
    let now = crate::time::now_ms();
    crate::usage::append_rate_limit(
        state,
        &crate::sessions::RateLimitInfo {
            status: REJECTED_RATE_STATUS.to_string(),
            is_using_overage: false,
            five_hour: Some(crate::sessions::RateLimitWindow {
                utilization: 1.0,
                resets_at: now / 1000 + 3600,
            }),
            seven_day: None,
            observed_at: now,
        },
    );
    let sessions = SessionManager::load_with_runner(
        state,
        Arc::new(crate::bus::Bus::new()),
        Arc::new(crate::projects::Projects::load(state)),
        Arc::new(aidp_core::process_runner::ScriptedProcessRunner::new()),
    );
    let guard = RateLimitGuard::new(sessions, state.join(VAULT_DIRECTORY), Arc::new(SystemClock));
    assert!(!guard.allows(LAUNCH_TEMPLATE));
}

const LAUNCH_KEY: &str = "LOC-7";
const LAUNCH_STEP: &str = "dev";
const LAUNCH_TEMPLATE: &str = "dev";
const LAUNCH_INPUTS: &str = "inputs.json";
const LAUNCH_PROJECT_ID: &str = "project-1";
const LAUNCH_REPO: &str = "repo";

fn pipeline_launch(autonomous: bool) -> PipelineLaunch {
    PipelineLaunch {
        key: LAUNCH_KEY.to_string(),
        ticket_key: LAUNCH_KEY.to_string(),
        provider: None,
        permission_mode: None,
        strict_mcp_config: None,
        mcp_config_id: None,
        mcp_servers: None,
        denied_shell_commands: Vec::new(),
        denied_mcp_tools: BTreeMap::new(),
        step: LAUNCH_STEP.to_string(),
        template: LAUNCH_TEMPLATE.to_string(),
        project: crate::models::ProjectRef {
            id: LAUNCH_PROJECT_ID.to_string(),
            path: PathBuf::from(LAUNCH_REPO),
            repo: LAUNCH_REPO.to_string(),
        },
        inputs_file: PathBuf::from(LAUNCH_INPUTS),
        auto_approve_gates: None,
        autonomous,
        headless: false,
        engine: None,
        agent_executor: AgentExecutor::Internal,
        project_checkout: false,
    }
}

/// The launch of an external-agents flow carries the mode to the session request.
#[test]
fn pipeline_start_request_carries_the_agent_executor() {
    let mut launch = pipeline_launch(false);
    assert_eq!(
        pipeline_start_request(&launch).agent_executor,
        AgentExecutor::Internal
    );
    launch.agent_executor = AgentExecutor::External;
    assert_eq!(
        pipeline_start_request(&launch).agent_executor,
        AgentExecutor::External
    );
}

/// An autonomous ticket step launches its session with the autonomous switch.
#[test]
fn pipeline_start_request_exports_autonomous_switch() {
    assert_eq!(
        pipeline_start_request(&pipeline_launch(true)).env,
        autonomous_env()
    );
}

/// A step that opted out of autonomy launches with an empty environment.
#[test]
fn pipeline_start_request_omits_switch_when_not_autonomous() {
    assert!(pipeline_start_request(&pipeline_launch(false))
        .env
        .is_empty());
}

/// A step of a headless flow launches a headless session; another step does not.
#[test]
fn pipeline_start_request_keeps_the_headless_launch_policy() {
    let headless = PipelineLaunch {
        headless: true,
        ..pipeline_launch(false)
    };
    assert!(pipeline_start_request(&headless).headless);
    assert!(!pipeline_start_request(&pipeline_launch(false)).headless);
}

const NATIVE_RUN_ID: &str = "run-1";
const NATIVE_STAGE_DIR: &str = "pr";
const NATIVE_VIEW_FIXTURE: &str = include_str!("../fixtures/flows/native_run_view.json");
const NATIVE_RUN_DIR_FIELD: &str = "runDir";
const NATIVE_STATUS_FIELD: &str = "status";

fn native_view(status: NativeRunStatus, run_dir: &Path) -> NativeRunView {
    let mut value: Value = serde_json::from_str(NATIVE_VIEW_FIXTURE).unwrap();
    value[NATIVE_RUN_DIR_FIELD] = json!(run_dir.to_string_lossy());
    value[NATIVE_STATUS_FIELD] = json!(status.as_str());
    serde_json::from_value(value).unwrap()
}

/// A pending or running native run reports Running to the executor.
#[test]
fn native_run_in_flight_is_running() {
    let dir = tempfile::tempdir().unwrap();
    for status in [
        NativeRunStatus::Pending,
        NativeRunStatus::Running,
        NativeRunStatus::WaitingGate,
    ] {
        assert_eq!(
            native_session_state(&native_view(status, dir.path())),
            SessionState::Running
        );
    }
}

/// A finished native run ends with its run id, run dir and the pr.json path.
#[test]
fn native_run_done_ends_with_outputs_from_the_run_dir() {
    let dir = tempfile::tempdir().unwrap();
    let stage = dir.path().join(NATIVE_STAGE_DIR);
    std::fs::create_dir_all(&stage).unwrap();
    let pr_file = stage.join(PR_FILE);
    std::fs::write(&pr_file, "{}").unwrap();
    let state = native_session_state(&native_view(NativeRunStatus::Done, dir.path()));
    let SessionState::Ended {
        run_status,
        outputs,
    } = state
    else {
        panic!("a done run must end");
    };
    assert_eq!(run_status, PipelineRunStatus::Done.as_str());
    assert_eq!(outputs[OUTPUT_RUN_ID], json!(NATIVE_RUN_ID));
    assert_eq!(outputs[OUTPUT_PR_FILE], json!(pr_file.to_string_lossy()));
    assert!(!outputs.contains_key(OUTPUT_VERIFY));
}

/// Failed, paused and gated native runs end as failed; a cancelled one as cancelled.
#[test]
fn native_run_terminal_statuses_map_to_run_statuses() {
    let dir = tempfile::tempdir().unwrap();
    let expected = [
        (NativeRunStatus::Failed, PipelineRunStatus::Failed),
        (NativeRunStatus::Paused, PipelineRunStatus::Failed),
        (NativeRunStatus::Cancelled, PipelineRunStatus::Cancelled),
    ];
    for (native, flow) in expected {
        match native_session_state(&native_view(native, dir.path())) {
            SessionState::Ended { run_status, .. } => assert_eq!(run_status, flow.as_str()),
            other => panic!("expected Ended, got {other:?}"),
        }
    }
}

/// The origin of a flow session names the business ticket, not the execution key.
#[test]
fn pipeline_start_request_references_the_ticket_key_in_the_origin() {
    let mut launch = pipeline_launch(false);
    launch.key = "GH-12-retry".to_string();
    launch.ticket_key = "GH-12".to_string();
    let origin = pipeline_start_request(&launch).origin.expect("an origin");
    assert_eq!(origin.kind, "flow");
    assert_eq!(origin.r#ref.as_deref(), Some("GH-12"));
    assert_eq!(origin.label.as_deref(), Some(LAUNCH_STEP));
}
