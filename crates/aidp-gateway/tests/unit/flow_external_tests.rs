//! External executor: jobs derived from the run, results mapped to machine events.

use crate::bus::Bus;
use crate::flows::agent_sessions_fake::{self, AgentSessions};
use crate::flows::executor::Ports;
use crate::flows::lock::{workspace_key, LocalLock, WorkspaceLock};
use crate::flows::registry::{
    ActionContext, AlwaysAllow, FlowAction, ProbeContext, ProbeOutcome, Registry, WaitProbe,
};
use crate::flows::{FlowEngine, FlowEngineConfig, FlowEngineError};
use crate::models::enums::{ExternalJobKind, FlowExecutorMode};
use crate::models::flow_execution_policy::FlowExecutionPolicy;
use crate::models::{ExternalResult, TicketFlowRequest};
use aidp_core::clock::Clock;
use aidp_core::id::IdGen;
use aidp_core::process_runner::ScriptedProcessRunner;
use aidp_flows::{ActionOutcome, FlowDef, FlowStatus, ManualRequest, Step, StepKind, StepStatus};
use aidp_testkit::TempWorkspace;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

const REPO: &str = "ext-repo";
const SERVICE: &str = "svc";
const ACTOR: &str = "ada";
const TICKET: &str = "EXT-1";
const COUNT_ACTION: &str = "count";
const WRITE_ACTION: &str = "write";
const PROBE: &str = "scripted";
const NOW_MS: i64 = 1_790_000_000_000;
const STEP_WORK: &str = "work";
const STEP_AFTER: &str = "after";
const STEP_WATCH: &str = "watch";
const SETTLE: Duration = Duration::from_millis(60);

struct CountingAction {
    runs: Arc<AtomicUsize>,
    writes: bool,
}

impl FlowAction for CountingAction {
    fn writes(&self) -> bool {
        self.writes
    }

    fn run(&self, _context: &ActionContext<'_>, _with: &BTreeMap<String, Value>) -> ActionOutcome {
        self.runs.fetch_add(1, Ordering::SeqCst);
        ActionOutcome::Ok(BTreeMap::new())
    }
}

struct CountingProbe(Arc<AtomicUsize>);

impl WaitProbe for CountingProbe {
    fn poll(&self, _context: &ProbeContext, _with: &BTreeMap<String, Value>) -> ProbeOutcome {
        self.0.fetch_add(1, Ordering::SeqCst);
        ProbeOutcome::pending(None)
    }
}

struct FixedClock;

impl Clock for FixedClock {
    fn now_utc(&self) -> chrono::DateTime<chrono::Utc> {
        chrono::DateTime::from_timestamp_millis(NOW_MS).expect("valid timestamp")
    }

    fn now_rfc3339(&self) -> String {
        self.now_utc().to_rfc3339()
    }

    fn now_ms(&self) -> i64 {
        NOW_MS
    }
}

struct Ids;

impl IdGen for Ids {
    fn new_run_id(&self, _clock: &dyn Clock) -> String {
        "id".to_string()
    }
}

struct Rig {
    _workspace: TempWorkspace,
    vault: PathBuf,
    lock: Arc<LocalLock>,
    engine: Arc<FlowEngine>,
    runs: Arc<AtomicUsize>,
    polls: Arc<AtomicUsize>,
    sessions: Arc<AgentSessions>,
}

fn engine_over(
    workspace: &TempWorkspace,
    vault: &std::path::Path,
    lock: &Arc<LocalLock>,
    runs: &Arc<AtomicUsize>,
    polls: &Arc<AtomicUsize>,
    sessions: &Arc<AgentSessions>,
) -> Arc<FlowEngine> {
    let mut registry = Registry::with_builtins();
    registry.register_action(
        COUNT_ACTION,
        Arc::new(CountingAction {
            runs: runs.clone(),
            writes: false,
        }),
    );
    registry.register_action(
        WRITE_ACTION,
        Arc::new(CountingAction {
            runs: runs.clone(),
            writes: true,
        }),
    );
    registry.register_probe(PROBE, Arc::new(CountingProbe(polls.clone())));
    let ports = Ports {
        sessions: sessions.clone(),
        budget: Arc::new(AlwaysAllow),
        lock: lock.clone(),
        bus: Arc::new(Bus::new()),
        clock: Arc::new(FixedClock),
        ids: Arc::new(Ids),
        runner: Arc::new(ScriptedProcessRunner::new()),
        lock_wait: Duration::from_millis(20),
    };
    FlowEngine::new(
        vault.to_path_buf(),
        workspace.child("state"),
        registry,
        ports,
        FlowEngineConfig::default(),
    )
}

impl Rig {
    fn new(tag: &str) -> Self {
        let workspace = TempWorkspace::new(tag);
        let vault = workspace.child("vault");
        std::fs::create_dir_all(&vault).unwrap();
        let lock = Arc::new(LocalLock::default());
        let runs = Arc::new(AtomicUsize::new(0));
        let polls = Arc::new(AtomicUsize::new(0));
        let sessions = Arc::new(AgentSessions::new(vault.clone(), REPO));
        let engine = engine_over(&workspace, &vault, &lock, &runs, &polls, &sessions);
        Rig {
            _workspace: workspace,
            vault,
            lock,
            engine,
            runs,
            polls,
            sessions,
        }
    }

    /// A new engine over the same files, as after a gateway restart.
    fn restart(&mut self) {
        self.engine = engine_over(
            &self._workspace,
            &self.vault,
            &self.lock,
            &self.runs,
            &self.polls,
            &self.sessions,
        );
    }

    fn start(&self, steps: Vec<Step>, executor: FlowExecutorMode) {
        self.start_with(steps, executor, false);
    }

    /// An external run that also hands its agent sessions to the host, with a run it started.
    fn start_agents(&self, steps: Vec<Step>) {
        self.start_with(steps, FlowExecutorMode::External, true);
        let mut runtime = self.engine.executor().store.load_runtime(REPO, TICKET);
        runtime.sessions.insert(
            "dev:1:run_pipeline".to_string(),
            agent_sessions_fake::RUN_ID.to_string(),
        );
        self.engine
            .executor()
            .store
            .save_runtime(REPO, TICKET, &runtime)
            .unwrap();
    }

    fn start_with(&self, steps: Vec<Step>, executor: FlowExecutorMode, external_agents: bool) {
        let def = FlowDef {
            id: "inline".to_string(),
            version: 1,
            steps,
            ..FlowDef::default()
        };
        let request = TicketFlowRequest {
            ticket: json!({ "key": TICKET, "service": SERVICE }),
            policy: Some(FlowExecutionPolicy {
                executor,
                external_agents,
                ..FlowExecutionPolicy::default()
            }),
            ..TicketFlowRequest::default()
        };
        self.engine.start_request_with_def(request, def).unwrap();
    }

    fn status(&self) -> FlowStatus {
        self.engine.get(TICKET).unwrap().status
    }

    fn step(&self, id: &str) -> StepStatus {
        self.engine.get(TICKET).unwrap().steps[id].status
    }

    fn post(
        &self,
        job: &str,
        result: ExternalResult,
    ) -> anyhow::Result<crate::models::ExternalReply> {
        self.engine.external_result(TICKET, job, result, ACTOR)
    }

    fn only_job_id(&self) -> String {
        let jobs = self.engine.external_jobs(TICKET).unwrap();
        assert_eq!(jobs.len(), 1, "{jobs:?}");
        jobs[0].id.clone()
    }
}

fn action(id: &str, name: &str) -> Step {
    Step {
        use_: Some(name.to_string()),
        ..Step::new(id, StepKind::Action)
    }
}

fn wait(id: &str, probe: &str) -> Step {
    Step {
        use_: Some(probe.to_string()),
        ..Step::new(id, StepKind::Wait)
    }
}

fn result(outcome: &str) -> ExternalResult {
    ExternalResult {
        outcome: outcome.to_string(),
        ..ExternalResult::default()
    }
}

fn with_reason(outcome: &str, reason: &str) -> ExternalResult {
    ExternalResult {
        reason: Some(reason.to_string()),
        ..result(outcome)
    }
}

fn two_actions() -> Vec<Step> {
    vec![
        action(STEP_WORK, COUNT_ACTION),
        action(STEP_AFTER, COUNT_ACTION),
    ]
}

fn error_kind(error: &anyhow::Error) -> &'static str {
    match error.downcast_ref::<FlowEngineError>() {
        Some(FlowEngineError::NotFound(_)) => "not_found",
        Some(FlowEngineError::Conflict { .. }) => "conflict",
        _ => "other",
    }
}

#[test]
fn an_action_effect_spawns_nothing_and_is_listed() {
    let rig = Rig::new("ext_action_listed");
    let mut step = action(STEP_WORK, COUNT_ACTION);
    step.with.insert("pr".to_string(), json!(7));
    rig.start(vec![step], FlowExecutorMode::External);
    std::thread::sleep(SETTLE);

    let jobs = rig.engine.external_jobs(TICKET).unwrap();

    assert_eq!(rig.runs.load(Ordering::SeqCst), 0);
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].kind, ExternalJobKind::Action);
    assert_eq!(jobs[0].name, COUNT_ACTION);
    assert_eq!(jobs[0].step, STEP_WORK);
    assert_eq!(jobs[0].with["pr"], json!(7));
    assert_eq!(jobs[0].ticket_key, TICKET);
    assert_eq!(jobs[0].repo, REPO);
    assert!(!jobs[0].manual && !jobs[0].local && !jobs[0].writes);
    let wire = serde_json::to_value(&jobs[0]).unwrap();
    assert_eq!(wire["ticketKey"], json!(TICKET));
    assert!(wire.get("startedMs").is_none());
}

#[test]
fn an_internal_run_lists_nothing_and_still_runs_its_actions() {
    let rig = Rig::new("ext_internal_unchanged");
    rig.start(
        vec![action(STEP_WORK, COUNT_ACTION)],
        FlowExecutorMode::Internal,
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while rig.status() != FlowStatus::Done {
        assert!(std::time::Instant::now() < deadline, "run did not finish");
        std::thread::sleep(Duration::from_millis(10));
    }

    assert_eq!(rig.runs.load(Ordering::SeqCst), 1);
    assert!(rig.engine.external_jobs(TICKET).unwrap().is_empty());
    let error = rig.post("work:1:run_action", result("ok")).unwrap_err();
    assert_eq!(error_kind(&error), "conflict");
}

#[test]
fn noop_and_fail_stay_in_the_gateway() {
    let rig = Rig::new("ext_noop_internal");
    rig.start(vec![action(STEP_WORK, "noop")], FlowExecutorMode::External);
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while rig.status() != FlowStatus::Done {
        assert!(std::time::Instant::now() < deadline, "run did not finish");
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(rig.engine.external_jobs(TICKET).unwrap().is_empty());
}

#[test]
fn an_ok_result_finishes_the_step_and_the_flow_goes_on() {
    let rig = Rig::new("ext_ok");
    rig.start(two_actions(), FlowExecutorMode::External);
    let job = rig.only_job_id();

    let mut posted = result("ok");
    posted.outputs = Some(json!({ "branch": "feat/EXT-1" }));
    let reply = rig.post(&job, posted).unwrap();

    assert!(!reply.duplicate);
    assert_eq!(rig.step(STEP_WORK), StepStatus::Done);
    let state = rig.engine.get(TICKET).unwrap();
    assert_eq!(
        state.steps[STEP_WORK].outputs["branch"],
        json!("feat/EXT-1")
    );
    let next = rig.only_job_id();
    assert_ne!(next, job);
    rig.post(&next, result("ok")).unwrap();
    assert_eq!(rig.status(), FlowStatus::Done);
}

#[test]
fn errors_map_to_failed_and_escalated_and_skip_moves_on() {
    let rig = Rig::new("ext_errors");
    rig.start(two_actions(), FlowExecutorMode::External);
    let job = rig.only_job_id();
    rig.post(&job, with_reason("skipped", "local_ticket"))
        .unwrap();
    assert_eq!(rig.step(STEP_WORK), StepStatus::Skipped);
    let next = rig.only_job_id();
    rig.post(&next, with_reason("business_error", "head moved"))
        .unwrap();
    assert_eq!(rig.status(), FlowStatus::Escalated);

    let technical = Rig::new("ext_technical");
    technical.start(two_actions(), FlowExecutorMode::External);
    let job = technical.only_job_id();
    technical
        .post(&job, with_reason("technical_error", "boom"))
        .unwrap();
    assert_eq!(technical.status(), FlowStatus::Failed);
}

#[test]
fn a_waiting_result_keeps_the_job_listed_and_replaces_the_outputs() {
    let rig = Rig::new("ext_waiting");
    rig.start(two_actions(), FlowExecutorMode::External);
    let job = rig.only_job_id();
    let mut waiting = with_reason("waiting_external", "ci_pending");
    waiting.outputs = Some(json!({ "run_id": 9 }));

    rig.post(&job, waiting.clone()).unwrap();

    assert_eq!(rig.step(STEP_WORK), StepStatus::WaitingExternal);
    assert_eq!(rig.only_job_id(), job);
    let state = rig.engine.get(TICKET).unwrap();
    assert_eq!(state.steps[STEP_WORK].outputs["run_id"], json!(9));
    let journal = rig.engine.journal(TICKET).unwrap().len();
    let again = rig.post(&job, waiting).unwrap();
    assert!(again.duplicate);
    assert_eq!(rig.engine.journal(TICKET).unwrap().len(), journal);
    rig.post(&job, result("ok")).unwrap();
    assert_eq!(rig.step(STEP_WORK), StepStatus::Done);
}

#[test]
fn a_replayed_result_is_flagged_and_another_digest_conflicts() {
    let rig = Rig::new("ext_idempotent");
    rig.start(two_actions(), FlowExecutorMode::External);
    let job = rig.only_job_id();
    rig.post(&job, result("ok")).unwrap();
    let applied = rig.engine.journal(TICKET).unwrap().len();

    let replay = rig.post(&job, result("ok")).unwrap();
    assert!(replay.duplicate);
    assert_eq!(replay.into_body()["duplicate"], json!(true));
    assert_eq!(rig.engine.journal(TICKET).unwrap().len(), applied);

    let other = rig
        .post(&job, with_reason("business_error", "no"))
        .unwrap_err();
    assert_eq!(error_kind(&other), "conflict");
    let unknown = rig.post("ghost:1:run_action", result("ok")).unwrap_err();
    assert_eq!(error_kind(&unknown), "not_found");
    let stale = rig.post("after:9:run_action", result("ok")).unwrap_err();
    assert_eq!(error_kind(&stale), "conflict");
}

#[test]
fn invalid_results_are_plain_errors() {
    let rig = Rig::new("ext_invalid");
    rig.start(two_actions(), FlowExecutorMode::External);
    let job = rig.only_job_id();
    let mut objectless = result("ok");
    objectless.outputs = Some(json!([1, 2]));

    for bad in [result("exploded"), result("skipped"), objectless] {
        let error = rig.post(&job, bad).unwrap_err();
        assert_eq!(error_kind(&error), "other", "{error}");
    }
    assert_eq!(rig.step(STEP_WORK), StepStatus::Running);
}

#[test]
fn a_probe_outcome_on_an_action_conflicts() {
    let rig = Rig::new("ext_mismatch");
    rig.start(two_actions(), FlowExecutorMode::External);
    let job = rig.only_job_id();

    let error = rig.post(&job, result("satisfied")).unwrap_err();

    assert_eq!(error_kind(&error), "conflict");
}

#[test]
fn a_paused_run_lists_nothing_and_refuses_results() {
    let rig = Rig::new("ext_paused");
    rig.start(two_actions(), FlowExecutorMode::External);
    let job = rig.only_job_id();

    rig.engine.pause(TICKET, ACTOR).unwrap();

    assert!(rig.engine.external_jobs(TICKET).unwrap().is_empty());
    let error = rig.post(&job, result("ok")).unwrap_err();
    assert_eq!(error_kind(&error), "conflict");
    rig.engine.resume(TICKET, ACTOR).unwrap();
    assert_eq!(rig.only_job_id(), job);
    rig.engine.abort(TICKET, ACTOR).unwrap();
    assert!(rig.engine.external_jobs(TICKET).unwrap().is_empty());
}

#[test]
fn a_busy_workspace_hides_a_writer_job() {
    let rig = Rig::new("ext_busy");
    rig.start(
        vec![action(STEP_WORK, WRITE_ACTION)],
        FlowExecutorMode::External,
    );
    let key = workspace_key(&rig.vault, REPO, TICKET);
    assert!(rig.lock.try_acquire(&key));

    assert!(rig.engine.external_jobs(TICKET).unwrap().is_empty());

    rig.lock.release(&key);
    let jobs = rig.engine.external_jobs(TICKET).unwrap();
    assert_eq!(jobs.len(), 1);
    assert!(jobs[0].writes);
}

#[test]
fn a_probe_is_listed_with_its_clock_and_its_state_round_trips() {
    let rig = Rig::new("ext_probe");
    rig.start(vec![wait(STEP_WATCH, PROBE)], FlowExecutorMode::External);
    std::thread::sleep(SETTLE);
    assert_eq!(rig.polls.load(Ordering::SeqCst), 0);
    let jobs = rig.engine.external_jobs(TICKET).unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].kind, ExternalJobKind::Probe);
    assert_eq!(jobs[0].name, PROBE);
    assert_eq!(jobs[0].now_ms, Some(NOW_MS));
    assert_eq!(jobs[0].started_ms, Some(NOW_MS));
    assert_eq!(jobs[0].state, None);
    let job = jobs[0].id.clone();

    let mut pending = result("pending");
    pending.state = Some(json!({ "seen": 2 }));
    rig.post(&job, pending.clone()).unwrap();

    assert_eq!(rig.step(STEP_WATCH), StepStatus::WaitingExternal);
    let listed = rig.engine.external_jobs(TICKET).unwrap();
    assert_eq!(listed[0].state, Some(json!({ "seen": 2 })));
    assert!(rig.post(&job, pending).unwrap().duplicate);
    let mismatch = rig.post(&job, result("ok")).unwrap_err();
    assert_eq!(error_kind(&mismatch), "conflict");

    let mut done = result("satisfied");
    done.outputs = Some(json!({ "threads": 0 }));
    rig.post(&job, done).unwrap();
    assert_eq!(rig.status(), FlowStatus::Done);
    let state = rig.engine.get(TICKET).unwrap();
    assert_eq!(state.steps[STEP_WATCH].outputs["threads"], json!(0));
    assert_eq!(rig.polls.load(Ordering::SeqCst), 0);
}

#[test]
fn a_manual_action_is_held_for_the_caller_and_cleared_by_its_result() {
    let rig = Rig::new("ext_manual");
    rig.start(vec![wait(STEP_WATCH, PROBE)], FlowExecutorMode::External);
    let request = ManualRequest::Action {
        name: COUNT_ACTION.to_string(),
        with: BTreeMap::from([("k".to_string(), json!("v"))]),
    };

    rig.engine.manual(TICKET, request, ACTOR).unwrap();

    std::thread::sleep(SETTLE);
    assert_eq!(rig.runs.load(Ordering::SeqCst), 0);
    let manual: Vec<_> = rig
        .engine
        .external_jobs(TICKET)
        .unwrap()
        .into_iter()
        .filter(|job| job.manual)
        .collect();
    assert_eq!(manual.len(), 1);
    assert_eq!(manual[0].id, "manual-0:0:run_action");
    assert_eq!(manual[0].with["k"], json!("v"));

    rig.post(&manual[0].id, result("ok")).unwrap();

    assert!(rig
        .engine
        .external_jobs(TICKET)
        .unwrap()
        .iter()
        .all(|job| !job.manual));
    assert!(rig.engine.journal(TICKET).unwrap().iter().any(|line| line
        .line
        .contains("manual effect manual-0:0:run_action finished")));
    assert!(rig.post(&manual[0].id, result("ok")).unwrap().duplicate);
}

#[test]
fn only_the_last_results_are_kept() {
    let rig = Rig::new("ext_ledger_limit");
    let steps: Vec<Step> = (0..70)
        .map(|index| action(&format!("s{index}"), COUNT_ACTION))
        .collect();
    rig.start(steps, FlowExecutorMode::External);
    let mut first = String::new();
    for index in 0..70 {
        let job = rig.only_job_id();
        if index == 0 {
            first = job.clone();
        }
        rig.post(&job, result("ok")).unwrap();
    }

    let error = rig.post(&first, result("ok")).unwrap_err();

    assert_eq!(error_kind(&error), "conflict");
    let runtime = rig.engine.executor().store.load_runtime(REPO, TICKET);
    assert_eq!(runtime.external_results.len(), 64);
}

fn manual_count_request() -> ManualRequest {
    ManualRequest::Action {
        name: COUNT_ACTION.to_string(),
        with: BTreeMap::new(),
    }
}

fn manual_jobs(rig: &Rig) -> Vec<crate::models::ExternalJob> {
    rig.engine
        .external_jobs(TICKET)
        .unwrap()
        .into_iter()
        .filter(|job| job.manual)
        .collect()
}

#[test]
fn a_manual_waiting_result_stays_listed_until_a_terminal_result() {
    let rig = Rig::new("ext_manual_waiting");
    rig.start(vec![wait(STEP_WATCH, PROBE)], FlowExecutorMode::External);
    rig.engine
        .manual(TICKET, manual_count_request(), ACTOR)
        .unwrap();
    let job = manual_jobs(&rig)[0].id.clone();
    let waiting = with_reason("waiting_external", "ci_pending");

    assert!(!rig.post(&job, waiting.clone()).unwrap().duplicate);

    assert_eq!(manual_jobs(&rig).len(), 1);
    let journal_lines = rig.engine.journal(TICKET).unwrap().len();
    assert!(rig.post(&job, waiting).unwrap().duplicate);
    assert_eq!(rig.engine.journal(TICKET).unwrap().len(), journal_lines);

    rig.post(&job, result("ok")).unwrap();

    assert!(manual_jobs(&rig).is_empty());
    assert!(rig.post(&job, result("ok")).unwrap().duplicate);
}

#[test]
fn a_probe_wait_record_survives_a_restart() {
    let mut rig = Rig::new("ext_probe_restart");
    rig.start(vec![wait(STEP_WATCH, PROBE)], FlowExecutorMode::External);
    let job = rig.only_job_id();
    let mut pending = result("pending");
    pending.state = Some(json!({ "seen": 3 }));
    rig.post(&job, pending).unwrap();

    rig.restart();
    rig.engine.executor().reconcile_all();
    std::thread::sleep(SETTLE);

    assert_eq!(rig.polls.load(Ordering::SeqCst), 0);
    let jobs = rig.engine.external_jobs(TICKET).unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].id, job);
    assert_eq!(jobs[0].state, Some(json!({ "seen": 3 })));
}

#[test]
fn a_manual_entry_survives_a_restart() {
    let mut rig = Rig::new("ext_manual_restart");
    rig.start(vec![wait(STEP_WATCH, PROBE)], FlowExecutorMode::External);
    rig.engine
        .manual(TICKET, manual_count_request(), ACTOR)
        .unwrap();
    let job = manual_jobs(&rig)[0].id.clone();

    rig.restart();
    rig.engine.executor().reconcile_all();
    std::thread::sleep(SETTLE);

    assert_eq!(rig.runs.load(Ordering::SeqCst), 0);
    let manual = manual_jobs(&rig);
    assert_eq!(manual.len(), 1);
    assert_eq!(manual[0].id, job);
}

fn agent_jobs_of(rig: &Rig) -> Vec<crate::models::ExternalJob> {
    rig.engine
        .external_jobs(TICKET)
        .unwrap()
        .into_iter()
        .filter(|job| job.kind == ExternalJobKind::Agent)
        .collect()
}

#[test]
fn the_agent_jobs_of_the_flow_runs_are_listed_beside_the_actions() {
    let rig = Rig::new("ext_agent_listed");
    rig.start_agents(vec![action(STEP_WORK, COUNT_ACTION)]);
    std::thread::sleep(SETTLE);

    let jobs = rig.engine.external_jobs(TICKET).unwrap();

    assert_eq!(jobs.len(), 2, "{jobs:?}");
    let agent = jobs
        .iter()
        .find(|job| job.kind == ExternalJobKind::Agent)
        .unwrap();
    assert_eq!(agent.id, agent_sessions_fake::job_id());
    assert_eq!(agent.step, "dev");
    assert_eq!(agent.name, "implement");
    assert_eq!(agent.ticket_key, TICKET);
    assert_eq!(agent.repo, REPO);
    assert!(agent.with.is_empty() && !agent.manual && agent.writes);
    let wire = serde_json::to_value(agent).unwrap();
    assert_eq!(wire["kind"], json!("agent"));
    assert_eq!(wire["agent"]["runId"], json!(agent_sessions_fake::RUN_ID));
    assert_eq!(wire["agent"]["jobKind"], json!("initial"));
    assert!(wire["agent"]["cwd"].is_string());
}

#[test]
fn an_action_job_has_no_agent_object() {
    let rig = Rig::new("ext_action_no_agent");
    rig.start(
        vec![action(STEP_WORK, COUNT_ACTION)],
        FlowExecutorMode::External,
    );
    std::thread::sleep(SETTLE);

    let wire = serde_json::to_value(&rig.engine.external_jobs(TICKET).unwrap()[0]).unwrap();

    assert!(wire.get("agent").is_none());
}

#[test]
fn a_paused_flow_lists_no_agent_job_but_still_takes_its_heartbeat_and_result() {
    let rig = Rig::new("ext_agent_paused");
    rig.start_agents(vec![action(STEP_WORK, COUNT_ACTION)]);
    let claim = rig
        .engine
        .agent_claim(TICKET, &agent_sessions_fake::job_id(), &Default::default())
        .unwrap();
    rig.engine.pause(TICKET, ACTOR).unwrap();

    assert!(rig.engine.external_jobs(TICKET).unwrap().is_empty());
    let beat = rig.engine.agent_heartbeat(
        TICKET,
        &agent_sessions_fake::job_id(),
        &crate::models::agent_heartbeat_request::AgentHeartbeatRequest {
            claim_token: claim.claim_token,
        },
    );
    assert!(beat.is_ok());
    let ack = rig.engine.agent_result(
        TICKET,
        &agent_sessions_fake::job_id(),
        &crate::models::agent_job_result::AgentJobResult::default(),
    );
    assert!(ack.unwrap().accepted);
}

#[test]
fn a_paused_flow_refuses_a_claim() {
    let rig = Rig::new("ext_agent_claim_paused");
    rig.start_agents(vec![action(STEP_WORK, COUNT_ACTION)]);
    rig.engine.pause(TICKET, ACTOR).unwrap();

    let error = rig
        .engine
        .agent_claim(TICKET, &agent_sessions_fake::job_id(), &Default::default())
        .unwrap_err();

    assert_eq!(error_kind(&error), "conflict");
    assert!(rig.sessions.calls.lock().unwrap().is_empty());
}

#[test]
fn a_claimed_agent_job_leaves_the_listing_and_holds_back_writer_manual_actions() {
    let rig = Rig::new("ext_agent_claimed");
    rig.start_agents(vec![action(STEP_WORK, COUNT_ACTION)]);
    rig.engine
        .manual(
            TICKET,
            ManualRequest::Action {
                name: WRITE_ACTION.to_string(),
                with: BTreeMap::new(),
            },
            ACTOR,
        )
        .unwrap();
    assert_eq!(agent_jobs_of(&rig).len(), 1);
    assert_eq!(manual_jobs(&rig).len(), 1);

    rig.engine
        .agent_claim(TICKET, &agent_sessions_fake::job_id(), &Default::default())
        .unwrap();

    assert!(agent_jobs_of(&rig).is_empty());
    assert!(manual_jobs(&rig).is_empty());
    rig.engine
        .agent_result(
            TICKET,
            &agent_sessions_fake::job_id(),
            &crate::models::agent_job_result::AgentJobResult::default(),
        )
        .unwrap();
    assert_eq!(manual_jobs(&rig).len(), 1);
}

#[test]
fn a_job_of_another_run_is_not_found_for_every_agent_call() {
    let rig = Rig::new("ext_agent_foreign");
    rig.start_agents(vec![action(STEP_WORK, COUNT_ACTION)]);
    let foreign = "agent:other-run:1";

    let claim = rig
        .engine
        .agent_claim(TICKET, foreign, &Default::default())
        .unwrap_err();
    let beat = rig
        .engine
        .agent_heartbeat(TICKET, foreign, &Default::default())
        .unwrap_err();
    let posted = rig
        .engine
        .agent_result(TICKET, foreign, &Default::default())
        .unwrap_err();
    let malformed = rig
        .engine
        .agent_claim(TICKET, "agent:garbled", &Default::default())
        .unwrap_err();

    for error in [claim, beat, posted, malformed] {
        assert_eq!(error_kind(&error), "not_found", "{error}");
    }
    assert!(rig.sessions.calls.lock().unwrap().is_empty());
}

#[test]
fn an_agent_call_for_an_unknown_flow_is_not_found() {
    let rig = Rig::new("ext_agent_unknown_flow");

    let error = rig
        .engine
        .agent_claim(
            "NOPE-1",
            &agent_sessions_fake::job_id(),
            &Default::default(),
        )
        .unwrap_err();

    assert_eq!(error_kind(&error), "not_found");
}
