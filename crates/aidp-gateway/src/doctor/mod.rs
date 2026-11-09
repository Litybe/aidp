//! The pipeline doctor: when a flow or a run breaks, the daemon opens an incident, runs the
//! `flow-doctor` investigation, executes an allowed action when it is safe, and reports.
//!
//! It runs on its own thread, never inside `reconcile_all`, and never holds a run's lock while it
//! calls the flow engine.

pub mod actions;
pub mod detect;
pub mod evidence;
pub mod http_routes;
pub mod policy;
pub mod ports;
pub mod store;

use crate::flows::executor::Executor;
use crate::flows::facts;
use crate::flows::registry::SessionState;
use crate::flows::registry::{FlowAction, SubprocessAction};
use crate::flows::FlowEngine;
use crate::models::constants::doctor::{
    AIDP_COMMENT, AIDP_TICKET, BODY_FILE_FLAG, COMMENT_FILE, DIAGNOSIS_FILE, DOCTOR_ACTOR,
    DOCTOR_DIR, DOCTOR_LOOP, DOCTOR_TEMPLATE, EVENT_DOCTOR_INCIDENT, INPUT_BUNDLE, INPUT_FEATURE,
    INPUT_INCIDENT, REPORT_FILE, SLICE,
};
use crate::models::constants::flow_facts::LIVE_STATUS_TIMEOUT;
use crate::models::enums::native_run_status::NativeRunStatus;
use crate::models::enums::{
    DoctorActionResult, DoctorIncidentStatus, DoctorLiveRun, FlowExecutorMode,
};
use crate::models::native_run_start::StartNativeRun;
use crate::models::native_run_view::NativeRunView;
use crate::models::{
    ActionContext, DoctorCandidate, DoctorDiagnosis, DoctorFlowSnapshot, DoctorIncident,
};
use crate::shutdown::Shutdown;
use aidp_flows::ActionOutcome;
use aidp_tickets::config::DoctorConfig;
use detect::DetectInput;
use policy::Decision;
use ports::{DoctorRuns, DoctorSettings, UnityProbe};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;
use store::IncidentStore;

const KEY_PLACEHOLDER: &str = "{key}";
const BODY_FILE_PLACEHOLDER: &str = "{body_file}";
const BODY_FILE_INPUT: &str = "body_file";
const ALL_GATES: &str = "*";
const FEATURE_PREFIX: &str = "doctor-";
const NONE_TEXT: &str = "-";
const PIPELINES_DIR: &str = "Pipelines";

/// What the doctor is built from.
pub struct DoctorDeps {
    pub engine: Arc<FlowEngine>,
    pub runs: Arc<dyn DoctorRuns>,
    pub settings: DoctorSettings,
    pub state_dir: PathBuf,
    pub unity: Option<UnityProbe>,
    /// Pause before the first pass of the thread, so the daemon finishes starting.
    pub first_delay: Duration,
    pub interval: Duration,
}

/// The doctor of one daemon.
pub struct Doctor {
    deps: DoctorDeps,
    store: IncidentStore,
    pass: Mutex<()>,
}

impl Doctor {
    pub fn new(deps: DoctorDeps) -> Arc<Self> {
        let store = IncidentStore::load(&deps.state_dir);
        Arc::new(Doctor {
            deps,
            store,
            pass: Mutex::new(()),
        })
    }

    /// The `doctor:` settings in force.
    pub fn settings(&self) -> DoctorConfig {
        (self.deps.settings)()
    }

    /// Every incident, oldest first.
    pub fn incidents(&self) -> Vec<DoctorIncident> {
        self.store.all()
    }

    /// The incidents of flow or ticket `key`, newest first.
    pub fn incidents_for(&self, key: &str) -> Vec<DoctorIncident> {
        self.store.for_key(key)
    }

    fn executor(&self) -> &Arc<Executor> {
        self.deps.engine.executor()
    }

    fn now(&self) -> String {
        self.executor().now()
    }

    fn now_ms(&self) -> i64 {
        self.executor().ports().clock.now_ms()
    }

    /// Starts the doctor's thread; it stops at `shutdown`. Its first pass waits `first_delay`,
    /// so detection never runs on the daemon's start path.
    pub fn spawn(self: &Arc<Self>, shutdown: Arc<Shutdown>) -> JoinHandle<()> {
        let doctor = self.clone();
        std::thread::spawn(move || {
            let _loop = crate::threads::enter_loop(DOCTOR_LOOP);
            let mut wait = doctor.deps.first_delay;
            let mut elapsed = Duration::ZERO;
            while !shutdown.is_set() {
                std::thread::sleep(SLICE);
                elapsed += SLICE;
                if elapsed < wait {
                    continue;
                }
                elapsed = Duration::ZERO;
                wait = doctor.deps.interval;
                crate::threads::guarded(DOCTOR_LOOP, || doctor.tick());
            }
        })
    }

    /// One pass: detect, follow the running investigation or start the next one.
    /// A pass already running makes this one return at once.
    pub fn tick(&self) {
        let Ok(_pass) = self.pass.try_lock() else {
            return;
        };
        let settings = self.settings();
        if !settings.enabled {
            return;
        }
        let candidates = self.detect(&settings);
        for candidate in &candidates {
            if let Some(incident) = self.store.open(candidate.clone(), &self.now()) {
                self.announce(
                    &incident,
                    &format!(
                        "incident {}: {}",
                        incident.candidate.trigger.as_str(),
                        incident.candidate.detail
                    ),
                );
            }
        }
        if let Some(open) = self.store.investigating() {
            self.follow(&open);
        }
        if self.store.investigating().is_none() {
            self.start_next(&settings, &candidates);
        }
    }

    /// The broken flows and runs of the moment, read from disk and from the live runs.
    pub fn detect(&self, settings: &DoctorConfig) -> Vec<DoctorCandidate> {
        let executor = self.executor();
        let flows: Vec<DoctorFlowSnapshot> = executor
            .store()
            .index()
            .into_values()
            .filter_map(|entry| {
                let state = executor.store().load_state(&entry.repo, &entry.key)?;
                let runtime = executor.store().load_runtime(&entry.repo, &entry.key);
                let external = runtime
                    .start_request
                    .as_ref()
                    .and_then(|request| request.policy.as_ref())
                    .is_some_and(|policy| policy.executor == FlowExecutorMode::External);
                Some(DoctorFlowSnapshot {
                    repo: entry.repo,
                    action_inflight: executor.has_inflight_action(&state.key),
                    state,
                    sessions: runtime.sessions,
                    external,
                })
            })
            .collect();
        let runs = self.deps.runs.list();
        let sessions = executor.ports().sessions.clone();
        let live = |id: &str| match self.deps.runs.get(id) {
            Some(view) => live_of_view(&view),
            None => match sessions.state(id) {
                SessionState::Running => DoctorLiveRun::Running,
                SessionState::Ended { .. } => DoctorLiveRun::Ended(None),
                SessionState::Gone => DoctorLiveRun::Gone,
            },
        };
        detect::detect(&DetectInput {
            flows: &flows,
            runs: &runs,
            live: &live,
            now_ms: self.now_ms(),
            stuck_minutes: settings.stuck_minutes,
        })
    }

    fn start_next(&self, settings: &DoctorConfig, candidates: &[DoctorCandidate]) {
        for incident in self.store.detected() {
            let id = incident.candidate.id.clone();
            if !candidates.iter().any(|candidate| candidate.id == id) {
                self.close(
                    &id,
                    DoctorIncidentStatus::NoAction,
                    "no longer broken before the investigation".to_string(),
                    false,
                );
                continue;
            }
            let attempts = self
                .store
                .attempts(&incident.candidate.ticket, self.now_ms());
            if policy::capped(attempts, settings.max_attempts_per_ticket) {
                let note = format!(
                    "{attempts} investigations of `{}` in 24 h: cap {} reached",
                    incident.candidate.ticket, settings.max_attempts_per_ticket
                );
                self.close(&id, DoctorIncidentStatus::Capped, note, true);
                continue;
            }
            if !self.executor().ports().budget.allows(DOCTOR_TEMPLATE) {
                let note = "investigation waits for the token budget".to_string();
                if incident.note.as_deref() != Some(note.as_str()) {
                    if let Some(updated) = self
                        .store
                        .update(&id, &self.now(), |row| row.note = Some(note.clone()))
                    {
                        self.announce(&updated, &note);
                    }
                }
                return;
            }
            self.investigate(&incident);
            return;
        }
    }

    fn investigate(&self, incident: &DoctorIncident) {
        let id = incident.candidate.id.clone();
        let bundle = self.bundle_dir(&incident.candidate);
        let (project_id, flow_dir, pr_status) = self.flow_context(&incident.candidate);
        let Some(project_id) = project_id.or_else(|| incident.candidate.project_id.clone()) else {
            self.close(
                &id,
                DoctorIncidentStatus::InvestigationFailed,
                "no project for the ticket".to_string(),
                true,
            );
            return;
        };
        let run = incident
            .candidate
            .run_id
            .as_deref()
            .and_then(|run_id| self.deps.runs.get(run_id));
        let unity = self
            .deps
            .unity
            .as_ref()
            .and_then(|probe| probe(&project_id));
        let source = evidence::EvidenceSource {
            candidate: &incident.candidate,
            flow_dir,
            run,
            state_dir: &self.deps.state_dir,
            pr_status,
            unity_status: unity,
        };
        if let Err(error) = evidence::write_bundle(&bundle, &source) {
            self.close(
                &id,
                DoctorIncidentStatus::InvestigationFailed,
                format!("evidence not written: {error}"),
                true,
            );
            return;
        }
        let request = StartNativeRun {
            project_id,
            template_id: DOCTOR_TEMPLATE.to_string(),
            inputs: BTreeMap::from([
                (INPUT_INCIDENT.to_string(), json!(id)),
                (INPUT_BUNDLE.to_string(), json!(bundle.to_string_lossy())),
                (
                    INPUT_FEATURE.to_string(),
                    json!(format!(
                        "{FEATURE_PREFIX}{}",
                        bundle_name(&incident.candidate.ticket)
                    )),
                ),
            ]),
            workspace: Some(bundle.clone()),
            autonomous: true,
            auto_approve_gates: vec![ALL_GATES.to_string()],
            ticket: Some(incident.candidate.ticket.clone()),
            ..StartNativeRun::default()
        };
        let request = self.with_session_policy(request, &incident.candidate);
        let now_ms = self.now_ms();
        match self.deps.runs.start(request) {
            Ok(run_id) => {
                let updated = self.store.update(&id, &self.now(), |row| {
                    row.status = DoctorIncidentStatus::Investigating;
                    row.investigation_run_id = Some(run_id.clone());
                    row.investigated_ms = Some(now_ms);
                    row.bundle = Some(bundle.to_string_lossy().to_string());
                    row.note = None;
                });
                if let Some(updated) = updated {
                    self.announce(&updated, &format!("investigation started (run {run_id})"));
                }
            }
            Err(error) => {
                self.store
                    .update(&id, &self.now(), |row| row.investigated_ms = Some(now_ms));
                self.close(
                    &id,
                    DoctorIncidentStatus::InvestigationFailed,
                    format!("investigation not started: {error}"),
                    true,
                );
            }
        }
    }

    /// The bundle directory of an incident, in the vault so the investigation may work there:
    /// `Pipelines/<repo>/<KEY>/doctor/<incident>/`, or `Pipelines/doctor/<incident>/` without a flow.
    fn bundle_dir(&self, candidate: &DoctorCandidate) -> PathBuf {
        let store = self.executor().store();
        let name = bundle_name(&candidate.id);
        match store.index_get(&candidate.key) {
            Some(entry) => store
                .ticket_dir(&entry.repo, &candidate.key)
                .join(DOCTOR_DIR)
                .join(name),
            None => store
                .vault_root()
                .join(PIPELINES_DIR)
                .join(DOCTOR_DIR)
                .join(name),
        }
    }

    /// The investigation runs its sessions the way the broken flow or run did: same provider,
    /// permission mode and headless flag, so it uses the agent the operator chose for the ticket.
    fn with_session_policy(
        &self,
        mut request: StartNativeRun,
        candidate: &DoctorCandidate,
    ) -> StartNativeRun {
        let executor = self.executor();
        let policy = executor
            .store()
            .index_get(&candidate.key)
            .and_then(|entry| {
                executor
                    .store()
                    .load_runtime(&entry.repo, &candidate.key)
                    .start_request
                    .and_then(|start| start.policy)
            });
        if let Some(policy) = policy {
            request.provider = policy.provider;
            request.permission_mode = policy.permission_mode;
            request.headless = policy.headless;
            return request;
        }
        let run = candidate
            .run_id
            .as_deref()
            .and_then(|id| self.deps.runs.get(id));
        if let Some(run) = run {
            request.provider = run.request.provider;
            request.permission_mode = run.request.permission_mode;
            request.headless = run.request.headless;
        }
        request
    }

    /// The project id, the flow directory and the live PR status of a flow incident.
    fn flow_context(
        &self,
        candidate: &DoctorCandidate,
    ) -> (Option<String>, Option<PathBuf>, Option<Value>) {
        let executor = self.executor();
        let Ok((repo, state)) = executor.load(&candidate.key) else {
            return (None, None, None);
        };
        let project = executor
            .ports()
            .sessions
            .project_for(&state.ticket)
            .map(|project| project.id);
        let flow_dir = executor.store().flow_dir(&repo, &candidate.key);
        let source = executor.registry().pr_status_source();
        let pr_status = facts::status_within(source, &candidate.ticket, LIVE_STATUS_TIMEOUT);
        (project, Some(flow_dir), pr_status)
    }

    /// Reads the end of the running investigation, then decides, acts and reports.
    fn follow(&self, incident: &DoctorIncident) {
        let id = incident.candidate.id.clone();
        let Some(run_id) = incident.investigation_run_id.as_deref() else {
            self.close(
                &id,
                DoctorIncidentStatus::InvestigationFailed,
                "no investigation run recorded".to_string(),
                true,
            );
            return;
        };
        let Some(view) = self.deps.runs.get(run_id) else {
            self.close(
                &id,
                DoctorIncidentStatus::InvestigationFailed,
                format!("investigation run {run_id} is gone"),
                true,
            );
            return;
        };
        match view.status {
            NativeRunStatus::Pending | NativeRunStatus::Running | NativeRunStatus::WaitingGate => {}
            NativeRunStatus::Done => self.conclude(incident, &view),
            other => self.close(
                &id,
                DoctorIncidentStatus::InvestigationFailed,
                format!(
                    "investigation run {run_id} ended {}: {}",
                    other.as_str(),
                    view.reason.as_deref().unwrap_or(NONE_TEXT)
                ),
                true,
            ),
        }
    }

    fn conclude(&self, incident: &DoctorIncident, view: &NativeRunView) {
        let id = incident.candidate.id.clone();
        let diagnosis = match read_diagnosis(&view.run_dir) {
            Ok(diagnosis) => diagnosis,
            Err(error) => {
                self.close(
                    &id,
                    DoctorIncidentStatus::InvestigationFailed,
                    format!("no readable {DIAGNOSIS_FILE}: {error}"),
                    true,
                );
                return;
            }
        };
        let report = find_file(&view.run_dir, REPORT_FILE)
            .and_then(|path| std::fs::read_to_string(path).ok());
        self.store.update(&id, &self.now(), |row| {
            row.diagnosis = Some(diagnosis.clone())
        });
        let incident = self.store.get(&id).unwrap_or_else(|| incident.clone());
        let (status, note) = match policy::decide(&diagnosis) {
            Decision::Refused(reason) => (DoctorIncidentStatus::Refused, reason),
            Decision::NeedsHuman(reason) => (DoctorIncidentStatus::NeedsHuman, reason),
            Decision::NoAction => (
                DoctorIncidentStatus::NoAction,
                "no action proposed".to_string(),
            ),
            Decision::Execute(action) => {
                if !self.store.claim_action(&id, action, &self.now()) {
                    (
                        DoctorIncidentStatus::Refused,
                        format!("action already executed for {}", store::action_key(&id)),
                    )
                } else {
                    let target = actions::ActionTarget {
                        engine: &self.deps.engine,
                        incident: &incident,
                        diagnosis: &diagnosis,
                    };
                    let result = actions::execute(&target, action);
                    let (status, ok, detail) = match result {
                        DoctorActionResult::Done(detail) => {
                            (DoctorIncidentStatus::Fixed, true, detail)
                        }
                        DoctorActionResult::Failed(detail) => {
                            (DoctorIncidentStatus::ActionFailed, false, detail)
                        }
                        DoctorActionResult::Refused(detail) => {
                            (DoctorIncidentStatus::Refused, false, detail)
                        }
                    };
                    let at = self.now();
                    self.store.update(&id, &at, |row| {
                        if let Some(outcome) = row.outcome.as_mut() {
                            outcome.ok = ok;
                            outcome.detail = detail.clone();
                            outcome.at = at.clone();
                        }
                    });
                    (status, format!("{} : {detail}", action.as_str()))
                }
            }
        };
        self.close_with_report(&id, status, note, report);
    }

    fn close(&self, id: &str, status: DoctorIncidentStatus, note: String, report: bool) {
        if report {
            self.close_with_report(id, status, note, None);
            return;
        }
        if let Some(updated) = self.store.update(id, &self.now(), |row| {
            row.status = status;
            row.note = Some(note.clone());
        }) {
            self.announce(&updated, &note);
        }
    }

    /// Closes the incident, posts the short issue comment, journals and publishes it.
    fn close_with_report(
        &self,
        id: &str,
        status: DoctorIncidentStatus,
        note: String,
        report: Option<String>,
    ) {
        let Some(closed) = self.store.update(id, &self.now(), |row| {
            row.status = status;
            row.note = Some(note.clone());
        }) else {
            return;
        };
        let commented = self.comment(&closed, report.as_deref());
        let closed = self
            .store
            .update(id, &self.now(), |row| row.commented = commented)
            .unwrap_or(closed);
        self.announce(&closed, &format!("{} ({note})", status.as_str()));
    }

    /// Posts the comment through `aidp ticket comment`; false when it could not be posted.
    fn comment(&self, incident: &DoctorIncident, report: Option<&str>) -> bool {
        let body = comment_body(incident, report);
        let dir = incident
            .bundle
            .as_ref()
            .map(PathBuf::from)
            .unwrap_or_else(|| self.bundle_dir(&incident.candidate));
        if std::fs::create_dir_all(&dir).is_err() {
            return false;
        }
        let file = dir.join(COMMENT_FILE);
        if std::fs::write(&file, body).is_err() {
            return false;
        }
        let action = SubprocessAction::new([
            AIDP_TICKET,
            AIDP_COMMENT,
            KEY_PLACEHOLDER,
            BODY_FILE_FLAG,
            BODY_FILE_PLACEHOLDER,
        ]);
        let runner = self.executor().ports().runner.clone();
        let context = ActionContext {
            key: &incident.candidate.ticket,
            workspace: None,
            runner: runner.as_ref(),
        };
        let with = BTreeMap::from([(BODY_FILE_INPUT.to_string(), json!(file.to_string_lossy()))]);
        match action.run(&context, &with) {
            ActionOutcome::Ok(_) => true,
            other => {
                tracing::warn!(incident = %incident.candidate.id, outcome = ?other, "doctor comment not posted");
                false
            }
        }
    }

    /// Publishes the incident on the bus and journals `line` on its flow when it has one.
    fn announce(&self, incident: &DoctorIncident, line: &str) {
        self.executor().ports().bus.emit(
            EVENT_DOCTOR_INCIDENT,
            serde_json::to_value(incident).unwrap_or_default(),
        );
        if self
            .executor()
            .store()
            .index_get(&incident.candidate.key)
            .is_some()
        {
            self.executor().journal(
                &incident.candidate.key,
                Some(DOCTOR_ACTOR),
                format!("doctor: {line}"),
            );
        }
    }
}

fn live_of_view(view: &NativeRunView) -> DoctorLiveRun {
    match view.status {
        NativeRunStatus::Pending | NativeRunStatus::Running | NativeRunStatus::WaitingGate => {
            DoctorLiveRun::Running
        }
        _ => DoctorLiveRun::Ended(view.ended_at.clone()),
    }
}

/// The first file named `name` in a stage directory of `run_dir`.
fn find_file(run_dir: &Path, name: &str) -> Option<PathBuf> {
    let mut stages: Vec<PathBuf> = std::fs::read_dir(run_dir)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    stages.sort();
    stages
        .into_iter()
        .map(|stage| stage.join(name))
        .find(|path| path.is_file())
}

/// The `diagnosis.json` the diagnose stage wrote.
pub fn read_diagnosis(run_dir: &Path) -> anyhow::Result<DoctorDiagnosis> {
    let file = find_file(run_dir, DIAGNOSIS_FILE)
        .ok_or_else(|| anyhow::anyhow!("{DIAGNOSIS_FILE} not found"))?;
    Ok(serde_json::from_str(&std::fs::read_to_string(file)?)?)
}

/// The short comment: cause, action, result, then the report stage's text when there is one.
pub fn comment_body(incident: &DoctorIncident, report: Option<&str>) -> String {
    let cause = incident
        .diagnosis
        .as_ref()
        .map(|diagnosis| diagnosis.cause.as_str())
        .filter(|cause| !cause.is_empty())
        .unwrap_or(incident.candidate.detail.as_str());
    let action = incident.outcome.as_ref().map_or_else(
        || NONE_TEXT.to_string(),
        |outcome| format!("`{}` : {}", outcome.action.as_str(), outcome.detail),
    );
    let mut body = format!(
        "**Docteur aidp** — incident `{}` ({})\n\n- Cause : {cause}\n- Action : {action}\n- Résultat : {} — {}\n",
        incident.candidate.id,
        incident.candidate.trigger.as_str(),
        incident.status.as_str(),
        incident.note.as_deref().unwrap_or(NONE_TEXT),
    );
    if let Some(report) = report.map(str::trim).filter(|text| !text.is_empty()) {
        body.push('\n');
        body.push_str(report);
        body.push('\n');
    }
    body
}

/// A file-safe folder name for incident `id`.
pub fn bundle_name(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
#[path = "../../tests/unit/doctor_tests.rs"]
mod tests;
