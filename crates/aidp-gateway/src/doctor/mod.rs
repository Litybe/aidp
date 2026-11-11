//! The pipeline doctor: when a flow or a run breaks, the daemon opens an incident, runs the
//! `flow-doctor` investigation, executes an allowed action when it is safe, and reports.
//!
//! It runs on its own thread, never inside `reconcile_all`, and never holds a run's lock while it
//! calls the flow engine.

pub mod actions;
pub mod actions_flow;
pub mod actions_git;
pub mod converge;
pub mod detect;
pub mod evidence;
pub mod http_routes;
pub mod policy;
pub mod ports;
pub mod preconditions;
pub mod store;
pub mod transient;

use crate::flows::executor::Executor;
use crate::flows::facts;
use crate::flows::registry::SessionState;
use crate::flows::registry::{FlowAction, SubprocessAction};
use crate::flows::FlowEngine;
use crate::models::constants::doctor::{
    AIDP_COMMENT, AIDP_TICKET, ATTEMPT_WINDOW_MS, BODY_FILE_FLAG, BUG_HISTORY_FILE, COMMENT_FILE,
    DIAGNOSIS_FILE, DOCTOR_ACTOR, DOCTOR_DIR, DOCTOR_LOOP, DOCTOR_TEMPLATE, EVENT_DOCTOR_INCIDENT,
    EVIDENCE_DIR, INPUT_BUNDLE, INPUT_FEATURE, INPUT_INCIDENT, INVESTIGATION_RETRY_DELAY_MS,
    MAX_ACTIONS_PER_INCIDENT, MAX_INVESTIGATION_FAILURES, MAX_TRANSIENT_RETRIES_PER_TICKET,
    OBSOLETE_NOTE, PARAM_PATTERN, PR_LAG_FILE, REPORT_FILE, REPORT_MAX_BYTES, SHARED_PATTERNS_FILE,
    SLICE, TEST_INFRA_FILE, TEST_TEMPLATE_FRAGMENT,
};
use crate::models::constants::flow_facts::LIVE_STATUS_TIMEOUT;
use crate::models::enums::native_run_status::NativeRunStatus;
use crate::models::enums::{
    DoctorAction, DoctorActionResult, DoctorCategory, DoctorIncidentStatus, DoctorLiveRun,
    DoctorTrigger, FlowExecutorMode,
};
use crate::models::native_run_start::StartNativeRun;
use crate::models::native_run_view::NativeRunView;
use crate::models::{
    ActionContext, DoctorCandidate, DoctorDiagnosis, DoctorFlowSnapshot, DoctorIncident,
    DoctorPrLag, DoctorSharedPattern,
};
use crate::shutdown::Shutdown;
use aidp_flows::ActionOutcome;
use aidp_tickets::config::DoctorConfig;
use aidp_tickets::models::constants::github::DEFAULT_KEY_PREFIX;
use aidp_tickets::store::TicketStore;
use converge::{Git, ReportText};
use detect::DetectInput;
use policy::Decision;
use ports::{DoctorRuns, DoctorSettings, HarnessRepos, UnityModals, UnityProbe};
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
const INVESTIGATION_RETRY_NOTE: &str = "the first investigation failed: a second one starts";
/// Largest stage file read to look for a passing fault.
const REPLY_MAX_BYTES: u64 = 64 * 1024;
const PIPELINES_DIR: &str = "Pipelines";

/// What the doctor is built from.
pub struct DoctorDeps {
    pub engine: Arc<FlowEngine>,
    pub runs: Arc<dyn DoctorRuns>,
    pub settings: DoctorSettings,
    pub state_dir: PathBuf,
    pub unity: Option<UnityProbe>,
    /// The channel to the modal windows of the Unity editor.
    pub unity_modals: Arc<dyn UnityModals>,
    /// Local clone of each harness repository, for `open_harness_pr`.
    pub harness_repos: HarnessRepos,
    /// Pause before the first pass of the thread, so the daemon finishes starting.
    pub first_delay: Duration,
    pub interval: Duration,
}

/// The doctor of one daemon.
pub struct Doctor {
    deps: DoctorDeps,
    store: IncidentStore,
    pass: Mutex<()>,
    /// Epoch milliseconds a stuck condition was first seen, by incident id.
    seen: Mutex<BTreeMap<String, i64>>,
}

impl Doctor {
    pub fn new(deps: DoctorDeps) -> Arc<Self> {
        let store = IncidentStore::load(&deps.state_dir);
        Arc::new(Doctor {
            deps,
            store,
            pass: Mutex::new(()),
            seen: Mutex::new(BTreeMap::new()),
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
        self.reopen_failed_investigations(&candidates);
        if let Some(open) = self.store.investigating() {
            self.follow(&open);
        }
        if self.store.investigating().is_none() {
            self.start_next(&settings, &candidates);
        }
    }

    /// An incident whose first investigation failed gets one more once the delay passed, while its
    /// failure is still live; a second failure stays final. The cap per ticket still applies.
    fn reopen_failed_investigations(&self, candidates: &[DoctorCandidate]) {
        let now_ms = self.now_ms();
        for incident in self.store.all() {
            let retry_due = incident.status == DoctorIncidentStatus::InvestigationFailed
                && incident.investigation_failures < MAX_INVESTIGATION_FAILURES
                && incident
                    .next_attempt_ms
                    .is_some_and(|not_before| now_ms >= not_before);
            let live = candidates
                .iter()
                .any(|candidate| candidate.id == incident.candidate.id);
            if !(retry_due && live) {
                continue;
            }
            let id = incident.candidate.id.clone();
            let updated = self.store.update(&id, &self.now(), |row| {
                row.status = DoctorIncidentStatus::Detected;
                row.investigation_run_id = None;
                row.next_attempt_ms = None;
                row.note = Some(INVESTIGATION_RETRY_NOTE.to_string());
            });
            if let Some(updated) = updated {
                self.announce(&updated, INVESTIGATION_RETRY_NOTE);
            }
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
        let now_ms = self.now_ms();
        let touched = Mutex::new(BTreeMap::new());
        let first_seen = |id: &str| {
            let at = *crate::sync::lock(&self.seen).get(id).unwrap_or(&now_ms);
            crate::sync::lock(&touched).insert(id.to_string(), at);
            at
        };
        let run_text = |view: &NativeRunView| self.run_text(view);
        let found = detect::detect(&DetectInput {
            flows: &flows,
            runs: &runs,
            live: &live,
            now_ms,
            stuck_minutes: settings.stuck_minutes,
            first_seen: &first_seen,
            run_text: &run_text,
        });
        // Only the conditions seen in this pass are remembered.
        *crate::sync::lock(&self.seen) = touched.into_inner().unwrap_or_default();
        found
    }

    /// The pause reason, the stage reasons and the small stage replies of a run: what tells a
    /// passing fault from a real failure.
    pub fn run_text(&self, view: &NativeRunView) -> String {
        let mut text = view.reason.clone().unwrap_or_default();
        for row in &view.stages {
            if let Some(reason) = &row.reason {
                text.push('\n');
                text.push_str(reason);
            }
        }
        if let Ok(stages) = std::fs::read_dir(&view.run_dir) {
            for stage in stages
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| path.is_dir())
            {
                for file in std::fs::read_dir(stage).into_iter().flatten().flatten() {
                    let path = file.path();
                    let small = file
                        .metadata()
                        .is_ok_and(|meta| meta.is_file() && meta.len() <= REPLY_MAX_BYTES);
                    // Replies and results only: never the stage's own prompt (`_<stage>.md`).
                    let named = path.extension().is_some_and(|ext| ext == "json")
                        || path.file_name().is_some_and(|name| name == "reply.md");
                    if small && named {
                        if let Ok(body) = aidp_core::text::read_text_lenient(&path) {
                            text.push('\n');
                            text.push_str(&body);
                        }
                    }
                }
            }
        }
        text
    }

    /// The passing fault behind an incident, if one is.
    fn transient_of(&self, incident: &DoctorIncident) -> Option<transient::Transient> {
        let mut text = incident.candidate.detail.clone();
        if let Some(view) = incident
            .candidate
            .run_id
            .as_deref()
            .and_then(|id| self.deps.runs.get(id))
        {
            text.push('\n');
            text.push_str(&self.run_text(&view));
        }
        transient::classify(&text)
    }

    /// True when the step of the incident belongs to the test lane: a project-checkout flow, or a
    /// step or template named for tests.
    fn test_lane(&self, candidate: &DoctorCandidate) -> bool {
        let Ok((_, state)) = self.executor().load(&candidate.key) else {
            return false;
        };
        if state.def.workspace == Some(aidp_flows::models::enums::WorkspaceMode::Project) {
            return true;
        }
        let step = candidate.step.as_deref().unwrap_or_default();
        let template = state
            .def
            .steps
            .iter()
            .find(|definition| definition.id == step)
            .and_then(|definition| definition.use_.clone())
            .unwrap_or_default();
        step.contains("test") || template.contains("test")
    }

    /// Closes the obsolete incidents, repairs the passing faults without a model, then starts
    /// the investigation of the first live incident: flows that are going first, the most recent
    /// first. A candidate that is no longer broken closes without investigation.
    fn start_next(&self, settings: &DoctorConfig, candidates: &[DoctorCandidate]) {
        let detected = self.store.detected();
        for incident in &detected {
            let id = incident.candidate.id.clone();
            if candidates.iter().any(|candidate| candidate.id == id) {
                continue;
            }
            if incident.actions.iter().any(|action| action.ok) {
                let note = format!(
                    "recovered after {}",
                    incident
                        .actions
                        .last()
                        .map_or(NONE_TEXT, |action| action.action.as_str())
                );
                self.close(&id, DoctorIncidentStatus::Fixed, note, true);
            } else {
                self.close(
                    &id,
                    DoctorIncidentStatus::NoAction,
                    OBSOLETE_NOTE.to_string(),
                    false,
                );
            }
        }
        let mut waiting_for_budget = false;
        // `candidates` is already ordered: live flows first, newest first.
        for candidate in candidates {
            let Some(incident) = self
                .store
                .get(&candidate.id)
                .filter(|row| row.status == DoctorIncidentStatus::Detected)
            else {
                continue;
            };
            let id = incident.candidate.id.clone();
            if let Some(fault) = self.transient_of(&incident) {
                self.repair_transient(&incident, fault);
                continue;
            }
            if waiting_for_budget || self.store.investigating().is_some() {
                continue;
            }
            let cap_incident = incident.candidate.trigger == DoctorTrigger::BugsNotConverging;
            if cap_incident {
                if let Some(covering) = self.covered_by_shared(&incident) {
                    let note = format!(
                        "covered by incident `{}` (shared pattern {})",
                        covering.candidate.id,
                        covering.shared_pattern.as_deref().unwrap_or(NONE_TEXT)
                    );
                    self.close(&id, DoctorIncidentStatus::NoAction, note, false);
                    continue;
                }
            }
            let attempts = self
                .store
                .attempts(&incident.candidate.ticket, self.now_ms());
            if !cap_incident && policy::capped(attempts, settings.max_attempts_per_ticket) {
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
                waiting_for_budget = true;
                continue;
            }
            self.investigate(&incident);
        }
    }

    /// Repairs a passing fault with no investigation: resumes the run, waiting longer between
    /// two resumptions; past the resumptions, requeues; past the actions, asks a human. None of
    /// it counts as an investigation of the ticket.
    fn repair_transient(&self, incident: &DoctorIncident, fault: transient::Transient) {
        let id = incident.candidate.id.clone();
        let now_ms = self.now_ms();
        if incident
            .next_attempt_ms
            .is_some_and(|not_before| now_ms < not_before)
        {
            return;
        }
        let diagnosis = DoctorDiagnosis {
            category: fault.category(),
            cause: fault.describe().to_string(),
            confidence: 1.0,
            summary: format!(
                "Panne passagère reconnue sans modèle : {}.",
                fault.describe()
            ),
            ..DoctorDiagnosis::default()
        };
        let test_lane = self.test_lane(&incident.candidate);
        let ticket_retries = self.store.retries(&incident.candidate.ticket, now_ms);
        let next = if ticket_retries >= MAX_TRANSIENT_RETRIES_PER_TICKET {
            None
        } else {
            policy::next_rung_with(
                &incident.actions,
                diagnosis.category,
                test_lane,
                policy::RungOptions {
                    fallback_model: self.settings().fallback_model.is_some(),
                },
            )
        };
        let Some(action) = next else {
            let note = format!(
                "{} persists after {} actions",
                fault.describe(),
                incident.actions.len()
            );
            self.store.update(&id, &self.now(), |row| {
                row.diagnosis = Some(diagnosis.clone())
            });
            self.close(&id, DoctorIncidentStatus::NeedsHuman, note, true);
            return;
        };
        let diagnosis = DoctorDiagnosis {
            action: action.as_str().to_string(),
            ..diagnosis
        };
        self.store.update(&id, &self.now(), |row| {
            row.diagnosis = Some(diagnosis.clone())
        });
        let Some(result) = self.attempt(&id, &diagnosis, action) else {
            self.close(
                &id,
                DoctorIncidentStatus::NeedsHuman,
                "the incident used its actions".to_string(),
                true,
            );
            return;
        };
        let retries = self.store.get(&id).map_or(0, |row| {
            row.actions
                .iter()
                .filter(|done| done.action == DoctorAction::RetryStep)
                .count()
        });
        let not_before = now_ms + policy::retry_delay_ms(retries.max(1));
        let note = format!(
            "{} : {} ({})",
            action.as_str(),
            result_detail(&result),
            fault.describe()
        );
        if let Some(updated) = self.store.update(&id, &self.now(), |row| {
            row.next_attempt_ms = Some(not_before);
            row.note = Some(note.clone());
        }) {
            self.announce(&updated, &note);
        }
    }

    /// Claims the next action of incident `id`, executes it and records its result; `None` when
    /// the incident has no action left. The key `<id>:action:<n>` is written before the action runs.
    fn attempt(
        &self,
        id: &str,
        diagnosis: &DoctorDiagnosis,
        action: DoctorAction,
    ) -> Option<DoctorActionResult> {
        let key = self.store.claim_action(id, action, &self.now())?;
        let incident = self.store.get(id)?;
        let settings = self.settings();
        let now_ms = self.now_ms();
        let target = actions::ActionTarget {
            engine: &self.deps.engine,
            runs: &self.deps.runs,
            incident: &incident,
            diagnosis,
            harness_repos: &self.deps.harness_repos,
            state_dir: &self.deps.state_dir,
            harness_ticket: settings.harness_ticket.as_deref(),
            rearm_allowed: !self
                .store
                .rearmed_recently(&incident.candidate.ticket, now_ms),
            settings: &settings,
            unity: self.deps.unity.as_ref(),
            unity_modals: &self.deps.unity_modals,
            budget_raises: self.store.budget_raises(&incident.candidate.ticket, now_ms),
        };
        let mut result = actions::execute(&target, action);
        let pr_url = match &result {
            DoctorActionResult::Done(done) if action == DoctorAction::OpenHarnessPr => {
                done.rsplit(' ').next().map(str::to_string)
            }
            _ => None,
        };
        if let DoctorActionResult::Done(done) = &result {
            if actions::cures_infrastructure(action, diagnosis.category) {
                let follow = actions::follow_up(&target, action);
                if follow.rearmed {
                    self.store
                        .update(id, &self.now(), |row| row.rearmed_ms = Some(now_ms));
                }
                result = DoctorActionResult::Done(format!("{done}; {}", follow.note));
            }
        }
        let ok = matches!(result, DoctorActionResult::Done(_));
        let detail = result_detail(&result);
        self.store.finish_action(id, &key, ok, &detail, &self.now());
        if let Some(url) = pr_url.filter(|_| ok) {
            self.store
                .update(id, &self.now(), |row| row.pr_url = Some(url.clone()));
        }
        Some(result)
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
        let convergence = self.convergence(&incident.candidate, pr_status.as_ref());
        let source = evidence::EvidenceSource {
            candidate: &incident.candidate,
            flow_dir,
            run,
            state_dir: &self.deps.state_dir,
            pr_status,
            unity_status: unity,
            convergence,
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
                    row.investigations += 1;
                    row.next_attempt_ms = None;
                    row.bundle = Some(bundle.to_string_lossy().to_string());
                    row.note = None;
                });
                if let Some(updated) = updated {
                    self.announce(&updated, &format!("investigation started (run {run_id})"));
                }
            }
            Err(error) => {
                self.store.update(&id, &self.now(), |row| {
                    row.investigated_ms = Some(now_ms);
                    row.investigations += 1;
                });
                self.close(
                    &id,
                    DoctorIncidentStatus::InvestigationFailed,
                    format!("investigation not started: {error}"),
                    true,
                );
            }
        }
    }

    /// The evidence about bugs that do not converge, computed from git and from the test reports:
    /// what each bug changed, the test-infrastructure files the branch changed, the lag behind
    /// `main`, and the errors several tickets share. A part that cannot be read says why.
    fn convergence(
        &self,
        candidate: &DoctorCandidate,
        pr_status: Option<&Value>,
    ) -> Vec<(&'static str, Value)> {
        let runner = self.executor().ports().runner.clone();
        let gathered = match actions::checkout_for(&self.deps.engine, &candidate.key) {
            Some(directory) => {
                let git = Git {
                    runner: runner.as_ref(),
                    cwd: &directory,
                };
                converge::gather(&git, &candidate.ticket, pr_status)
            }
            None => Err("no checkout for the project of the ticket".to_string()),
        };
        let (bugs, infra, lag) = match gathered {
            Ok(found) => (found.bugs, found.infra, found.lag),
            Err(reason) => (
                Vec::new(),
                Vec::new(),
                DoctorPrLag {
                    unavailable: Some(reason),
                    ..DoctorPrLag::default()
                },
            ),
        };
        let shared = self.shared_patterns();
        [
            (BUG_HISTORY_FILE, serde_json::to_value(bugs)),
            (TEST_INFRA_FILE, serde_json::to_value(infra)),
            (PR_LAG_FILE, serde_json::to_value(lag)),
            (SHARED_PATTERNS_FILE, serde_json::to_value(shared)),
        ]
        .into_iter()
        .map(|(name, value)| (name, value.unwrap_or(Value::Null)))
        .collect()
    }

    /// The errors found in the test reports of at least two different tickets over the last 24
    /// hours. A bug counts for its parent ticket, so a ticket and its own bugs are not two.
    pub fn shared_patterns(&self) -> Vec<DoctorSharedPattern> {
        let now_ms = self.now_ms();
        let reports: Vec<ReportText> = self
            .deps
            .runs
            .list()
            .into_iter()
            .filter(|view| {
                view.template_id != DOCTOR_TEMPLATE
                    && view.template_id.contains(TEST_TEMPLATE_FRAGMENT)
                    && detect::millis_of(&view.started_at)
                        .is_some_and(|at| now_ms - at < ATTEMPT_WINDOW_MS)
            })
            .filter_map(|view| {
                let ticket = view
                    .request
                    .ticket
                    .clone()
                    .or_else(|| view.request.target.clone())?;
                Some(ReportText {
                    ticket,
                    run_id: view.run_id.clone(),
                    text: report_text(&view),
                })
            })
            .collect();
        let tickets = TicketStore::open(self.executor().store().vault_root()).ok();
        let family = |ticket: &str| {
            tickets
                .as_ref()
                .and_then(|store| parent_key_of(store, ticket))
                .unwrap_or_else(|| ticket.to_string())
        };
        converge::shared_patterns(&reports, &family)
    }

    /// The incident that already handles, for every ticket it touches, a pattern `incident`'s ticket carries.
    fn covered_by_shared(&self, incident: &DoctorIncident) -> Option<DoctorIncident> {
        let tickets = TicketStore::open(self.executor().store().vault_root()).ok();
        let ticket = tickets
            .as_ref()
            .and_then(|store| parent_key_of(store, &incident.candidate.ticket))
            .unwrap_or_else(|| incident.candidate.ticket.clone());
        let patterns: Vec<String> = self
            .shared_patterns()
            .into_iter()
            .filter(|pattern| pattern.tickets.contains(&ticket))
            .map(|pattern| pattern.key)
            .collect();
        if patterns.is_empty() {
            return None;
        }
        self.store
            .covering(&patterns, self.now_ms())
            .filter(|covering| covering.candidate.id != incident.candidate.id)
    }

    /// A shared harness failure is one incident for every ticket it touches: the pattern and the
    /// tickets come from the bundle the daemon computed, the diagnosis only picks the pattern.
    fn record_shared(&self, incident: &DoctorIncident, diagnosis: &DoctorDiagnosis) {
        let Some(bundle) = incident.bundle.as_deref() else {
            return;
        };
        let file = Path::new(bundle)
            .join(EVIDENCE_DIR)
            .join(SHARED_PATTERNS_FILE);
        let patterns: Vec<DoctorSharedPattern> = std::fs::read_to_string(file)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        let named = diagnosis
            .params
            .get(PARAM_PATTERN)
            .and_then(Value::as_str)
            .and_then(|key| patterns.iter().find(|pattern| pattern.key == key));
        let Some(pattern) = named.or_else(|| patterns.first()) else {
            return;
        };
        let mut affected = pattern.tickets.clone();
        if !affected.contains(&incident.candidate.ticket) {
            affected.insert(0, incident.candidate.ticket.clone());
        }
        let key = pattern.key.clone();
        self.store
            .update(&incident.candidate.id, &self.now(), |row| {
                row.shared_pattern = Some(key.clone());
                row.affected = affected.clone();
            });
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
        if diagnosis.category == DoctorCategory::SharedHarnessFailure {
            self.record_shared(incident, &diagnosis);
        }
        let (status, note) = match policy::decide(&diagnosis) {
            Decision::Refused(reason) => (DoctorIncidentStatus::Refused, reason),
            Decision::NeedsHuman(reason) if diagnosis.category == DoctorCategory::RealDefect => (
                DoctorIncidentStatus::NeedsHuman,
                format!("{}; {reason}", policy::real_defect_summary(&diagnosis)),
            ),
            Decision::NeedsHuman(reason) => (DoctorIncidentStatus::NeedsHuman, reason),
            Decision::NoAction => (
                DoctorIncidentStatus::NoAction,
                format!("no action: {}", diagnosis.cause.trim()),
            ),
            Decision::Execute(action) => self.execute_with_escalation(&id, &diagnosis, action),
        };
        self.close_with_report(&id, status, note, report);
    }

    /// Runs the proposed action, then, when it did not work and is one of the rungs of the
    /// escalation, the next rungs: resumption, requeue (test or dev), bug ticket, a human. Three
    /// actions at most, each under its own idempotency key.
    fn execute_with_escalation(
        &self,
        id: &str,
        diagnosis: &DoctorDiagnosis,
        proposed: DoctorAction,
    ) -> (DoctorIncidentStatus, String) {
        let mut action = proposed;
        let mut tried = 0usize;
        loop {
            let Some(result) = self.attempt(id, diagnosis, action) else {
                return (
                    DoctorIncidentStatus::Refused,
                    format!("incident `{id}` used its {MAX_ACTIONS_PER_INCIDENT} actions"),
                );
            };
            tried += 1;
            let line = format!("{} : {}", action.as_str(), result_detail(&result));
            let (status, detail) = match result {
                DoctorActionResult::Done(_) => return (DoctorIncidentStatus::Fixed, line),
                DoctorActionResult::Failed(_) => (DoctorIncidentStatus::ActionFailed, line),
                DoctorActionResult::Refused(_) if action == DoctorAction::OpenHarnessPr => {
                    // A patch that does not apply is noted, then a human takes over.
                    (DoctorIncidentStatus::NeedsHuman, line)
                }
                DoctorActionResult::Refused(_) => (DoctorIncidentStatus::Refused, line),
            };
            if action == DoctorAction::CreateBugTicket
                && diagnosis.category == DoctorCategory::RealDefect
            {
                return (
                    DoctorIncidentStatus::NeedsHuman,
                    format!("{}; {detail}", policy::real_defect_summary(diagnosis)),
                );
            }
            // Only a reversible action that did not work climbs the ladder, and a patch that
            // does not apply falls back to a bug issue on the harness ticket; a refused bug
            // ticket goes to a human.
            let can_escalate = proposed.is_reversible() || policy::escalates(diagnosis.category);
            let done = self
                .store
                .get(id)
                .map(|row| row.actions)
                .unwrap_or_default();
            let next = if can_escalate {
                let test_lane = self
                    .store
                    .get(id)
                    .is_some_and(|row| self.test_lane(&row.candidate));
                policy::next_rung_with(
                    &done,
                    diagnosis.category,
                    test_lane,
                    policy::RungOptions {
                        fallback_model: self.settings().fallback_model.is_some(),
                    },
                )
                .filter(|rung| diagnosis.confidence >= policy::required_confidence(*rung))
            } else {
                None
            };
            match next {
                Some(rung) => action = rung,
                None if tried > 1 => {
                    return (
                        DoctorIncidentStatus::NeedsHuman,
                        format!("{tried} actions did not settle it; last {detail}"),
                    )
                }
                None => return (status, detail),
            }
        }
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
        let now_ms = self.now_ms();
        let Some(closed) = self.store.update(id, &self.now(), |row| {
            row.status = status;
            row.note = Some(note.clone());
            if status == DoctorIncidentStatus::InvestigationFailed {
                row.investigation_failures += 1;
                row.next_attempt_ms = (row.investigation_failures < MAX_INVESTIGATION_FAILURES)
                    .then_some(now_ms + INVESTIGATION_RETRY_DELAY_MS);
            }
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
        let with = BTreeMap::from([(BODY_FILE_INPUT.to_string(), json!(file.to_string_lossy()))]);
        let mut tickets = vec![incident.candidate.ticket.as_str()];
        tickets.extend(
            incident
                .affected
                .iter()
                .map(String::as_str)
                .filter(|ticket| *ticket != incident.candidate.ticket),
        );
        let mut posted = true;
        for ticket in tickets {
            let context = ActionContext {
                key: ticket,
                workspace: None,
                runner: runner.as_ref(),
            };
            match action.run(&context, &with) {
                ActionOutcome::Ok(_) => {}
                other => {
                    tracing::warn!(incident = %incident.candidate.id, outcome = ?other, "doctor comment not posted");
                    posted = false;
                }
            }
        }
        posted
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

fn result_detail(result: &DoctorActionResult) -> String {
    match result {
        DoctorActionResult::Done(detail)
        | DoctorActionResult::Failed(detail)
        | DoctorActionResult::Refused(detail) => detail.clone(),
    }
}

/// The text of a test run: its pause reason, its stage reasons and its reports and results. The
/// stage prompts (`_<stage>.md`) are not part of it.
fn report_text(view: &NativeRunView) -> String {
    let mut text = view.reason.clone().unwrap_or_default();
    for row in &view.stages {
        if let Some(reason) = &row.reason {
            text.push('\n');
            text.push_str(reason);
        }
    }
    let Ok(stages) = std::fs::read_dir(&view.run_dir) else {
        return text;
    };
    for stage in stages
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
    {
        for file in std::fs::read_dir(stage).into_iter().flatten().flatten() {
            let path = file.path();
            let small = file
                .metadata()
                .is_ok_and(|meta| meta.is_file() && meta.len() <= REPORT_MAX_BYTES);
            let readable = path
                .extension()
                .is_some_and(|ext| ext == "md" || ext == "json")
                && !path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().starts_with('_'));
            if small && readable {
                if let Ok(body) = aidp_core::text::read_text_lenient(&path) {
                    text.push('\n');
                    text.push_str(&body);
                }
            }
        }
    }
    text
}

/// The key of the parent a bug ticket belongs to, `None` for a ticket that is not a bug.
fn parent_key_of(store: &TicketStore, key: &str) -> Option<String> {
    let (ticket, _, _) = store.get(key).ok()?;
    let meta = ticket.github?;
    let parent = meta.parent?;
    Some(match store.config().github.repo(&meta.repo) {
        Some(repo) => repo.key_of(parent),
        None => format!("{DEFAULT_KEY_PREFIX}-{parent}"),
    })
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
    Ok(serde_json::from_str(&aidp_core::text::read_text_lenient(
        file,
    )?)?)
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
