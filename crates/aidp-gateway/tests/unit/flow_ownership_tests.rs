//! The ownership adapter and the ticket status a delivery run writes.

use super::ActiveFlowOwnership;
use crate::bus::Bus;
use crate::flows::executor::Ports;
use crate::flows::lock::LocalLock;
use crate::flows::registry::{
    ActionContext, BudgetGuard, FlowAction, PipelineLaunch, ProjectRef, Registry, SessionPort,
    SessionStart, SessionState,
};
use crate::flows::{FlowEngine, FlowEngineConfig};
use aidp_core::clock::SystemClock;
use aidp_core::id::SystemIdGen;
use aidp_core::process_runner::ScriptedProcessRunner;
use aidp_flows::ActionOutcome;
use aidp_integrations::models::flow_ownership::FlowOwnership;
use aidp_testkit::TempWorkspace;
use aidp_tickets::cli::{new_ticket, BodySource, NewTicket};
use aidp_tickets::ledger;
use aidp_tickets::model::{Actor, Status};
use aidp_tickets::store::TicketStore;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// The ticket value a flow start receives.
#[derive(serde::Serialize)]
struct TicketFixture<'a> {
    key: &'a str,
    service: &'a str,
}

const REPO: &str = "ownership-repo";
const SERVICE: &str = "svc";
const FLOW: &str = "perso-delivery";
const OTHER_FLOW: &str = "test-flow";
const SETTING_WITH_PIPELINE: &str = "with_pipeline";
const COUNT_ACTION: &str = "count";
const STEP_APPROVE: &str = "approve";
const ACTOR: &str = "alice";
const SUMMARY: &str = "Ship it";
const KEY_FIELD: &str = "key";
const UNKNOWN_KEY: &str = "NOPE-1";
const WAIT_LIMIT: Duration = Duration::from_secs(10);
const WAIT_POLL: Duration = Duration::from_millis(10);
const LOCK_WAIT: Duration = Duration::from_millis(60);
const LEDGER_LIMIT: usize = 20;

struct CountAction;

impl FlowAction for CountAction {
    fn writes(&self) -> bool {
        false
    }

    fn run(&self, _context: &ActionContext<'_>, _with: &BTreeMap<String, Value>) -> ActionOutcome {
        ActionOutcome::Ok(BTreeMap::new())
    }
}

struct AllowAll;

impl BudgetGuard for AllowAll {
    fn allows(&self, _template: &str) -> bool {
        true
    }
}

struct NoSessions(PathBuf);

impl SessionPort for NoSessions {
    fn project_for(&self, _ticket: &Value) -> Option<ProjectRef> {
        Some(ProjectRef {
            id: "p1".to_string(),
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

struct Harness {
    _workspace: TempWorkspace,
    vault: PathBuf,
    engine: Arc<FlowEngine>,
}

impl Harness {
    fn new(tag: &str) -> Self {
        let workspace = TempWorkspace::new(tag);
        let vault = workspace.child("vault");
        std::fs::create_dir_all(&vault).unwrap();
        let mut registry = Registry::with_builtins();
        registry.register_action(COUNT_ACTION, Arc::new(CountAction));
        let ports = Ports {
            sessions: Arc::new(NoSessions(vault.clone())),
            budget: Arc::new(AllowAll),
            lock: Arc::new(LocalLock::default()),
            bus: Arc::new(Bus::new()),
            clock: Arc::new(SystemClock),
            ids: Arc::new(SystemIdGen),
            runner: Arc::new(ScriptedProcessRunner::new()),
            lock_wait: LOCK_WAIT,
        };
        let config = FlowEngineConfig {
            flow_dirs: vec![PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/flows")],
            ..FlowEngineConfig::default()
        };
        let engine = FlowEngine::new(
            vault.clone(),
            workspace.child("state"),
            registry,
            ports,
            config,
        );
        Harness {
            _workspace: workspace,
            vault,
            engine,
        }
    }

    fn create_ticket(&self) -> String {
        let store = TicketStore::open(&self.vault).unwrap();
        let created = new_ticket(
            &store,
            NewTicket {
                service: SERVICE.to_string(),
                summary: SUMMARY.to_string(),
                issue_type: None,
                priority: None,
                body: BodySource::Template,
                labels: None,
                template: None,
                track: None,
                epic: None,
                allow_unknown_service: true,
            },
        )
        .unwrap();
        created[KEY_FIELD].as_str().unwrap().to_string()
    }

    fn ticket_status(&self, key: &str) -> Status {
        let store = TicketStore::open(&self.vault).unwrap();
        store.get(key).unwrap().0.status
    }

    fn wait_for_status(&self, key: &str, wanted: Status) {
        let deadline = Instant::now() + WAIT_LIMIT;
        while self.ticket_status(key) != wanted {
            assert!(Instant::now() < deadline, "timed out waiting for {wanted}");
            std::thread::sleep(WAIT_POLL);
        }
    }

    fn start(&self, key: &str) {
        let ticket = serde_json::to_value(TicketFixture {
            key,
            service: SERVICE,
        })
        .unwrap();
        self.engine.start(ticket, Some(FLOW), None).unwrap();
    }
}

#[test]
fn it_active_run_owns_its_ticket_and_an_unknown_key_does_not() {
    let harness = Harness::new("flow_ownership_active");
    let key = harness.create_ticket();
    let ownership = ActiveFlowOwnership::new(harness.engine.clone());
    assert!(!ownership.owns(&key));
    harness.start(&key);
    assert!(ownership.owns(&key));
    assert!(!ownership.owns(UNKNOWN_KEY));
}

#[test]
fn it_aborted_run_no_longer_owns_its_ticket() {
    let harness = Harness::new("flow_ownership_aborted");
    let key = harness.create_ticket();
    harness.start(&key);
    harness.engine.abort(&key, ACTOR).unwrap();
    let ownership = ActiveFlowOwnership::new(harness.engine.clone());
    assert!(!ownership.owns(&key));
}

#[test]
fn it_ticket_follows_the_run_with_the_flow_actor() {
    let harness = Harness::new("flow_ownership_status");
    let key = harness.create_ticket();
    harness.start(&key);
    harness.wait_for_status(&key, Status::InReview);

    let store = TicketStore::open(&harness.vault).unwrap();
    let ticket = store.get(&key).unwrap().0;
    assert_eq!(ticket.autopilot.flow.as_deref(), Some(FLOW));

    harness.engine.approve(&key, STEP_APPROVE, ACTOR).unwrap();
    harness.wait_for_status(&key, Status::Done);

    let expected = vec![Status::InProgress, Status::InReview, Status::Done];
    let deadline = Instant::now() + WAIT_LIMIT;
    loop {
        let flow_targets = flow_targets(&harness.vault, &key);
        if flow_targets == expected {
            break;
        }
        assert!(Instant::now() < deadline, "ledger holds {flow_targets:?}");
        std::thread::sleep(WAIT_POLL);
    }
}

/// A run of a flow other than the default one moves its ticket too.
#[test]
fn it_ticket_follows_a_run_of_any_flow() {
    let harness = Harness::new("flow_ownership_any_flow");
    let key = harness.create_ticket();
    let ticket = serde_json::to_value(TicketFixture {
        key: &key,
        service: SERVICE,
    })
    .unwrap();
    let settings = serde_json::json!({ SETTING_WITH_PIPELINE: false });
    harness
        .engine
        .start(ticket, Some(OTHER_FLOW), Some(settings))
        .unwrap();
    harness.wait_for_status(&key, Status::InProgress);
    let store = TicketStore::open(&harness.vault).unwrap();
    let ticket = store.get(&key).unwrap().0;
    assert_eq!(ticket.autopilot.flow.as_deref(), Some(OTHER_FLOW));
}

fn flow_targets(vault: &std::path::Path, key: &str) -> Vec<Status> {
    ledger::read(vault, Some(key), LEDGER_LIMIT)
        .iter()
        .filter(|event| event.actor == Actor::Flow)
        .map(|event| event.to)
        .collect()
}
