//! Launch mode of the dispatcher (session or flow) and the PR references of the merge probe.

use super::*;
use crate::flows::executor::Ports;
use crate::flows::lock::LocalLock;
use crate::flows::registry::{
    BudgetGuard, PipelineLaunch, ProjectRef, Registry, SessionPort, SessionStart, SessionState,
};
use crate::flows::{FlowEngineConfig, DEFAULT_FLOW};
use aidp_core::id::SystemIdGen;
use aidp_core::process_runner::ScriptedProcessRunner;
use aidp_testkit::TempWorkspace;
use aidp_tickets::model::IssueType;
use serde::Serialize;
use serde_json::json;
use std::time::Duration;

const CONFIG_FLOW: &str = include_str!("../fixtures/dispatcher_launch/config_flow.yaml");
const CONFIG_SESSION: &str = include_str!("../fixtures/dispatcher_launch/config_session.yaml");
const FLOW_FIXTURE_DIR: &str = "tests/fixtures/dispatcher_launch";
const TODO_FOLDER: &str = "TODO";
const CONFIG_NAME: &str = "config.yaml";
const SERVICE: &str = "svc";
const REPO: &str = "launch-repo";
const KEY: &str = "FLOW-1";
const OTHER_KEY: &str = "FLOW-2";
const FIX_PREFIX: &str = "fix/";
const FEAT_PREFIX: &str = "feat/";
const RECORDED_NUMBER: u64 = 42;
const TICKET_FIELD: &str = "ticket";
const LOCK_WAIT: Duration = Duration::from_millis(60);

/// The slice of a `gh pr view` reply the merge probe reads.
#[derive(Serialize)]
struct PrViewFixture {
    state: GhPrState,
    number: u64,
}

fn pr_view(state: GhPrState) -> Value {
    serde_json::to_value(PrViewFixture {
        state,
        number: RECORDED_NUMBER,
    })
    .unwrap()
}

struct AllowAll;

impl BudgetGuard for AllowAll {
    fn allows(&self, _template: &str) -> bool {
        true
    }
}

struct FixedProject(PathBuf);

impl SessionPort for FixedProject {
    fn project_for(&self, _ticket: &Value) -> Option<ProjectRef> {
        Some(ProjectRef {
            id: SERVICE.to_string(),
            path: self.0.clone(),
            repo: REPO.to_string(),
        })
    }

    fn start(&self, _launch: &PipelineLaunch) -> anyhow::Result<SessionStart> {
        Ok(SessionStart::Busy)
    }

    fn state(&self, _session_id: &str) -> SessionState {
        SessionState::Gone
    }

    fn cancel(&self, _session_id: &str) {}
}

struct Rig {
    dispatcher: Dispatcher,
    engine: Arc<FlowEngine>,
    store: TicketStore,
    _vault: TempWorkspace,
    _state: TempWorkspace,
}

fn rig(tag: &str, config: &str) -> Rig {
    let vault = TempWorkspace::new(&format!("dispatcher-launch-vault-{tag}"));
    let state = TempWorkspace::new(&format!("dispatcher-launch-state-{tag}"));
    let todo = vault.path().join(TODO_FOLDER);
    std::fs::create_dir_all(&todo).unwrap();
    std::fs::write(todo.join(CONFIG_NAME), config).unwrap();
    let bus = Arc::new(Bus::new());
    let projects = Arc::new(Projects::load(state.path()));
    let sessions = SessionManager::load(state.path(), bus.clone(), projects.clone());
    let ports = Ports {
        sessions: Arc::new(FixedProject(vault.path().to_path_buf())),
        budget: Arc::new(AllowAll),
        lock: Arc::new(LocalLock::default()),
        bus: bus.clone(),
        clock: Arc::new(SystemClock),
        ids: Arc::new(SystemIdGen),
        runner: Arc::new(ScriptedProcessRunner::new()),
        lock_wait: LOCK_WAIT,
    };
    let engine_config = FlowEngineConfig {
        flow_dirs: vec![PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(FLOW_FIXTURE_DIR)],
        ..FlowEngineConfig::default()
    };
    let engine = FlowEngine::new(
        vault.path().to_path_buf(),
        state.child("flows"),
        Registry::with_builtins(),
        ports,
        engine_config,
    );
    let dispatcher = Dispatcher {
        vault_root: vault.path().to_path_buf(),
        sessions,
        launcher: None,
        flows: Mutex::new(None),
        projects,
        bus,
        state: Mutex::new(DispatcherState::default()),
        skill_root: None,
        last_sync: Mutex::new(None),
        catalog_cache: Mutex::new(None),
        tick_lock: Mutex::new(()),
        warned_no_vault: AtomicBool::new(false),
        first_tick: AtomicBool::new(true),
        worktree_ops: WorktreeOps::system(),
    };
    let store = TicketStore::open(vault.path()).unwrap();
    Rig {
        dispatcher,
        engine,
        store,
        _vault: vault,
        _state: state,
    }
}

fn seed_ticket(store: &TicketStore, key: &str) {
    seed_ticket_of_type(store, key, IssueType::Task);
}

fn seed_ticket_of_type(store: &TicketStore, key: &str, issue_type: IssueType) {
    use aidp_tickets::model::{Autopilot, Priority, Source, Ticket};
    let now = Utc::now();
    let mut ticket = Ticket {
        key: key.to_string(),
        source: Source::Local,
        r#type: issue_type,
        summary: key.to_string(),
        status: Status::Todo,
        priority: Priority::Medium,
        priority_rank: Priority::Medium.rank(),
        service: SERVICE.to_string(),
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
    };
    store
        .save(&mut ticket, "", &serde_yaml::Mapping::new())
        .unwrap();
}

fn launch_of(key: &str) -> plan::Launch {
    plan::Launch {
        key: key.to_string(),
        service: SERVICE.to_string(),
        template: String::new(),
        track: String::new(),
        demand: plan::Demand::Issue { number: 0 },
        manual: false,
        detail: None,
        flow: None,
        test: false,
    }
}

fn reload(store: &TicketStore, key: &str) -> aidp_tickets::model::Ticket {
    store.get(key).unwrap().0
}

fn skip_reason(outcome: LaunchAttemptOutcome) -> String {
    match outcome {
        LaunchAttemptOutcome::Skipped { reason, .. } => reason.to_string(),
        LaunchAttemptOutcome::Launched(id) => panic!("expected a skip, launched {id}"),
    }
}

#[test]
fn launch_defaults_to_session_and_never_starts_a_flow() {
    let rig = rig("session", CONFIG_SESSION);
    rig.dispatcher.attach_flows(rig.engine.clone());
    seed_ticket(&rig.store, KEY);

    assert_eq!(
        rig.store.config().dispatcher.launch,
        DispatcherLaunch::Session
    );
    assert!(rig.engine.list(None, None).is_empty());
    assert_eq!(reload(&rig.store, KEY).autopilot.flow, None);
}

#[test]
fn flow_launch_starts_a_flow_run_and_no_session() {
    let rig = rig("flow", CONFIG_FLOW);
    rig.dispatcher.attach_flows(rig.engine.clone());
    seed_ticket(&rig.store, KEY);
    assert_eq!(rig.store.config().dispatcher.launch, DispatcherLaunch::Flow);

    let outcome = rig
        .dispatcher
        .start_delivery_flow(&rig.store, &launch_of(KEY));

    let LaunchAttemptOutcome::Launched(reference) = outcome else {
        panic!("a flow launch must be launched");
    };
    assert_eq!(reference, format!("{FLOW_SESSION_PREFIX}{KEY}"));
    assert!(rig.engine.get(KEY).is_ok());
    assert!(rig.dispatcher.sessions.list(None, None, 0).is_empty());
    let ticket = reload(&rig.store, KEY);
    assert_eq!(ticket.status, Status::InProgress);
    assert_eq!(ticket.autopilot.flow.as_deref(), Some(DEFAULT_FLOW));
    assert_eq!(ticket.autopilot.session_id, None);
}

#[test]
fn flow_launch_without_an_engine_is_skipped() {
    let rig = rig("no-engine", CONFIG_FLOW);
    seed_ticket(&rig.store, KEY);

    let outcome = rig
        .dispatcher
        .start_delivery_flow(&rig.store, &launch_of(KEY));

    assert_eq!(
        skip_reason(outcome),
        aidp_tickets::SkipReason::FlowUnavailable.as_str()
    );
    assert_eq!(reload(&rig.store, KEY).status, Status::Todo);
}

#[test]
fn a_ticket_with_an_active_flow_is_not_launched_again() {
    let rig = rig("active", CONFIG_FLOW);
    rig.dispatcher.attach_flows(rig.engine.clone());
    seed_ticket(&rig.store, KEY);
    let launch = launch_of(KEY);
    assert!(matches!(
        rig.dispatcher.start_delivery_flow(&rig.store, &launch),
        LaunchAttemptOutcome::Launched(_)
    ));

    let second = rig.dispatcher.start_delivery_flow(&rig.store, &launch);

    assert_eq!(
        skip_reason(second),
        aidp_tickets::SkipReason::FlowActive.as_str()
    );
    assert_eq!(rig.engine.list(None, None).len(), 1);
}

#[test]
fn merge_probe_resolves_feat_then_fix_branches() {
    let references = merge_probe_references(OTHER_KEY, None, false);

    assert_eq!(
        references,
        vec![
            format!("{FEAT_PREFIX}{OTHER_KEY}"),
            format!("{FIX_PREFIX}{OTHER_KEY}")
        ]
    );
}

#[test]
fn merge_probe_prefers_the_recorded_number_of_a_flow_owned_ticket() {
    let flow_owned = merge_probe_references(KEY, Some(RECORDED_NUMBER), true);
    let session_owned = merge_probe_references(KEY, Some(RECORDED_NUMBER), false);

    assert_eq!(flow_owned[0], RECORDED_NUMBER.to_string());
    assert_eq!(flow_owned.len(), 3);
    assert_eq!(session_owned.len(), 2);
}

#[test]
fn a_fix_branch_pr_is_found_when_the_feat_branch_has_none() {
    let references = merge_probe_references(KEY, None, false);
    let fix_branch = format!("{FIX_PREFIX}{KEY}");

    let view = first_pr_view(&references, |reference| {
        (reference == fix_branch).then(|| pr_view(GhPrState::Merged))
    })
    .expect("the fix branch resolves");

    assert!(matches!(
        final_pr_state(&view),
        Some(lifecycle::PrState::Merged)
    ));
}

#[test]
fn an_open_pr_has_no_final_state() {
    assert!(final_pr_state(&pr_view(GhPrState::Open)).is_none());
    assert!(matches!(
        final_pr_state(&pr_view(GhPrState::Closed)),
        Some(lifecycle::PrState::Closed)
    ));
}

#[test]
fn feeding_a_flow_merge_emits_pr_merged_without_touching_the_status() {
    let rig = rig("merge-feed", CONFIG_FLOW);
    seed_ticket(&rig.store, KEY);
    let ticket = reload(&rig.store, KEY);
    let events = rig.dispatcher.bus.subscribe();

    rig.dispatcher
        .feed_flow_merge(&ticket, &json!({ NUMBER_KEY: RECORDED_NUMBER }));

    let event = events.try_recv().expect("a pr.merged event");
    assert_eq!(event[FIELD_TYPE], EVENT_PR_MERGED);
    assert_eq!(event[FIELD_DATA][TICKET_FIELD], KEY);
    assert_eq!(reload(&rig.store, KEY).status, Status::Todo);
}

/// Starts the flow of a ticket of `issue_type` and returns the branch it recorded.
fn recorded_branch(tag: &str, issue_type: IssueType) -> Option<String> {
    let rig = rig(tag, CONFIG_FLOW);
    rig.dispatcher.attach_flows(rig.engine.clone());
    seed_ticket_of_type(&rig.store, KEY, issue_type);
    rig.dispatcher
        .start_delivery_flow(&rig.store, &launch_of(KEY));
    reload(&rig.store, KEY).autopilot.branch
}

#[test]
fn a_bug_ticket_records_its_fix_branch() {
    assert_eq!(
        recorded_branch("bug-branch", IssueType::Bug),
        Some(format!("{FIX_PREFIX}{KEY}"))
    );
}

#[test]
fn a_task_ticket_records_its_feature_branch() {
    assert_eq!(
        recorded_branch("task-branch", IssueType::Task),
        Some(format!("{FEAT_PREFIX}{KEY}"))
    );
}

#[test]
fn flow_ticket_reasons_serialize_to_their_wire_strings() {
    let reasons = [
        FlowTicketReason::Started,
        FlowTicketReason::StatusSync,
        FlowTicketReason::Escalated,
        FlowTicketReason::Failed,
    ];
    for reason in reasons {
        assert_eq!(
            serde_json::to_value(reason).unwrap(),
            json!(reason.as_str())
        );
    }
    assert_eq!(FlowTicketReason::Started.as_str(), "flow-started");
    assert_eq!(FlowTicketReason::Failed.as_str(), "flow failed");
}

const CONFIG_LANES: &str = include_str!("../fixtures/dispatcher_launch/config_lanes.yaml");
const ROUTING_LANES: &str = include_str!("../fixtures/dispatcher_launch/routing_lanes.yaml");

/// A rig whose project is named `SERVICE`, with the lane config and routing, flows attached.
fn lane_rig(tag: &str) -> Rig {
    let rig = rig(tag, CONFIG_LANES);
    std::fs::write(
        rig._vault.path().join(TODO_FOLDER).join("routing.yaml"),
        ROUTING_LANES,
    )
    .unwrap();
    let project = rig._state.child(SERVICE);
    std::fs::create_dir_all(&project).unwrap();
    rig.dispatcher.projects.add(project.to_str().unwrap());
    rig.dispatcher.attach_flows(rig.engine.clone());
    rig
}

fn seed_in_lane(rig: &Rig, key: &str, phase: model::Phase, status: Status) {
    use aidp_tickets::model::Autopilot;
    seed_ticket(&rig.store, key);
    let (mut ticket, body, extra) = rig.store.get(key).unwrap();
    ticket.status = status;
    ticket.flow = Some("game-code".to_string());
    ticket.autopilot = Autopilot::default();
    ticket.autopilot.set_phase(phase, Utc::now());
    rig.store.save(&mut ticket, &body, &extra).unwrap();
}

fn launched_keys(report: &Value) -> Vec<String> {
    report["launched"]
        .as_array()
        .unwrap()
        .iter()
        .map(|key| key.as_str().unwrap().to_string())
        .collect()
}

fn skipped_reason(report: &Value, key: &str) -> Option<String> {
    report["skipped"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["key"] == key)
        .map(|entry| entry["reason"].as_str().unwrap().to_string())
}

#[test]
fn a_to_test_issue_starts_its_test_flow_and_becomes_testing() {
    let rig = lane_rig("lane-test-flow");
    seed_in_lane(&rig, "T-1", model::Phase::ToTest, Status::InProgress);

    let report = rig.dispatcher.tick_local("test");

    assert_eq!(launched_keys(&report), ["T-1"]);
    assert_eq!(rig.engine.get("T-1").unwrap().def.id, "game-test-code");
    let ticket = reload(&rig.store, "T-1");
    assert_eq!(ticket.autopilot.phase(), model::Phase::Testing);
    assert_eq!(ticket.status, Status::InProgress);
}

#[test]
fn two_to_test_issues_pass_one_after_the_other() {
    let rig = lane_rig("lane-serial");
    seed_in_lane(&rig, "T-1", model::Phase::ToTest, Status::InProgress);
    seed_in_lane(&rig, "T-2", model::Phase::ToTest, Status::InProgress);

    let first = rig.dispatcher.tick_local("test");
    assert_eq!(launched_keys(&first).len(), 1);
    let (running, waiting) = if launched_keys(&first) == ["T-1"] {
        ("T-1", "T-2")
    } else {
        ("T-2", "T-1")
    };
    assert_eq!(
        skipped_reason(&first, waiting).as_deref(),
        Some("no-test-slot")
    );

    let blocked = rig.dispatcher.tick_local("test");
    assert!(launched_keys(&blocked).is_empty());
    assert_eq!(
        skipped_reason(&blocked, waiting).as_deref(),
        Some("no-test-slot")
    );

    rig.engine.abort(running, "test").unwrap();
    let next = rig.dispatcher.tick_local("test");
    assert_eq!(launched_keys(&next), [waiting]);
    assert_eq!(
        reload(&rig.store, waiting).autopilot.phase(),
        model::Phase::Testing
    );
}

#[test]
fn three_dev_flows_run_in_parallel_while_a_test_runs() {
    let rig = lane_rig("lane-parallel");
    seed_in_lane(&rig, "T-1", model::Phase::ToTest, Status::InProgress);
    for key in ["D-1", "D-2", "D-3", "D-4"] {
        seed_in_lane(&rig, key, model::Phase::Dev, Status::Todo);
    }

    let report = rig.dispatcher.tick_local("test");

    let launched = launched_keys(&report);
    assert!(
        launched.contains(&"T-1".to_string()),
        "{launched:?} {report}"
    );
    assert_eq!(
        launched.iter().filter(|key| key.starts_with("D-")).count(),
        3,
        "{report}"
    );
    let waiting: Vec<&str> = ["D-1", "D-2", "D-3", "D-4"]
        .into_iter()
        .filter(|key| !launched.contains(&key.to_string()))
        .collect();
    assert_eq!(waiting.len(), 1);
    assert_eq!(
        skipped_reason(&report, waiting[0]).as_deref(),
        Some("no-slot")
    );
    let (dev, test) = rig.engine.lane_load();
    // The fixture dev flow parks on a human gate at once: parked flows hold no slot.
    assert_eq!(dev, 0);
    assert_eq!(test.get(SERVICE), Some(&1));
    for key in launched.iter().filter(|key| key.starts_with("D-")) {
        assert_eq!(reload(&rig.store, key).autopilot.phase(), model::Phase::Dev);
    }
    assert_eq!(report["slotsUsed"], 0);
}

#[test]
fn a_test_flow_does_not_take_a_dev_slot() {
    let rig = lane_rig("lane-no-dev-slot");
    seed_in_lane(&rig, "T-1", model::Phase::ToTest, Status::InProgress);
    rig.dispatcher.tick_local("test");
    seed_in_lane(&rig, "D-1", model::Phase::Dev, Status::Todo);

    let report = rig.dispatcher.tick_local("test");

    assert_eq!(launched_keys(&report), ["D-1"]);
    assert_eq!(report["slotsMax"], 3);
    assert_eq!(report["slotsUsed"], 0);
}

#[test]
fn a_forced_start_is_not_short_circuited_by_an_open_pr() {
    // `adoptable` is false for a forced reason, a manual launch or a flow launch.
    let rig = lane_rig("lane-force");
    seed_in_lane(&rig, "D-1", model::Phase::Dev, Status::Todo);
    let mut launch = launch_of("D-1");
    launch.manual = true;
    launch.flow = Some("game-code".to_string());
    let updated: HashMap<String, DateTime<Utc>> =
        [("D-1".to_string(), reload(&rig.store, "D-1").updated)].into();

    let outcome = rig.dispatcher.execute_launch_reason(
        &rig.store,
        &launch,
        &updated,
        FORCED_OVER_PAUSE_REASON,
    );

    assert!(matches!(outcome, LaunchAttemptOutcome::Launched(_)));
    assert!(rig.engine.get("D-1").is_ok());
}
