//! Flows executor: runs the [`aidp_flows`] effects, persists runs in the
//! vault and exposes [`FlowEngine`].

pub mod action_processes;
pub mod actions;
pub mod events;
pub mod executor;
pub mod external;
pub mod external_agents;
pub mod facts;
pub mod http_routes;
pub mod lock;
pub mod ownership;
pub mod pipeline_inputs;
pub mod probes;
pub mod process_runner;
pub mod recovery;
pub mod registry;
pub mod step_outputs;
pub mod store;
pub mod ticket_snapshot;
pub mod unity_wait;

#[cfg(test)]
#[path = "../../tests/unit/agent_sessions_fake.rs"]
pub(crate) mod agent_sessions_fake;

#[cfg(test)]
#[path = "../../tests/unit/lane_slot_tests.rs"]
mod lane_slot_tests;

use crate::inbox::InboxEntry;
use crate::models::constants::flow_facts::{VIEW_PR, VIEW_SESSIONS};
use crate::models::constants::ticket_flow::{FLOW_ROUTE_MISSING, REPLY_CODE_KEY, REPLY_ERROR_KEY};
use crate::models::enums::ApiStatus;
pub use crate::models::enums::{FlowEngineError, InboxAction, InboxKind};
pub use crate::models::FlowEngineConfig;
use crate::models::{ExternalJob, ExternalReply, ExternalResult, FlowReply, TicketFlowRequest};
use crate::routes::FLOWS;
use crate::sync::lock as mutex_lock;
use aidp_flows::{
    validate, Event, FlowDef, FlowError, FlowState, HumanDecision, ManualRequest, RejectReason,
    FLOW_LABEL,
};
use executor::{Executor, Ports};
use registry::Registry;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use store::{is_active, FlowStore, IndexEntry, JournalLine};
use tiny_http::Method;

/// Flow started when the request names none.
pub const DEFAULT_FLOW: &str = "perso-delivery";
/// Folder of the skill root holding the flow definitions.
pub const FLOW_TEMPLATES_DIR: &str = "templates/flows";
/// Longest a writing action waits for its workspace before the step retries later.
pub const FLOW_LOCK_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

const FLOW_EXTENSION: &str = "yaml";
const TICKET_KEY: &str = "key";
const VIEW_REPO: &str = "repo";
const VIEW_INBOX: &str = "inbox";
const INBOX_URGENCY: u8 = 0;
const SETTINGS_NOT_OBJECT: &str = "settings must be a JSON object";
const KEY_MISSING: &str = "ticket.key is required";

/// Reply of a flow route lookup that found no handler.
fn route_missing_reply() -> FlowReply {
    let status = ApiStatus::Internal;
    FlowReply {
        status: status.status(),
        body: json!({ REPLY_ERROR_KEY: FLOW_ROUTE_MISSING, REPLY_CODE_KEY: status.code() }),
    }
}

/// Starts the delivery flow of `ticket` through the route of `POST /api/flows`.
/// A lookup that finds no handler answers an internal error.
pub fn start_ticket_flow(
    engine: &Arc<FlowEngine>,
    ticket: Value,
    mut request: TicketFlowRequest,
) -> FlowReply {
    request.ticket = ticket;
    request.flow.get_or_insert_with(|| DEFAULT_FLOW.to_string());
    let body = serde_json::to_value(&request).unwrap_or_default();
    http_routes::handle(engine, &Method::Post, FLOWS, &HashMap::new(), &body)
        .unwrap_or_else(route_missing_reply)
}

/// Reads the run of `key` through the route of `GET /api/flows/:key`.
/// A lookup that finds no handler answers an internal error.
pub fn read_ticket_flow(engine: &Arc<FlowEngine>, key: &str) -> FlowReply {
    let path = format!("{FLOWS}/{key}");
    http_routes::handle(engine, &Method::Get, &path, &HashMap::new(), &Value::Null)
        .unwrap_or_else(route_missing_reply)
}

/// Maps a machine refusal: unknown step 404, invalid choice 400, else 409.
pub(crate) fn map_flow_error(error: FlowError) -> anyhow::Error {
    match error {
        FlowError::Rejected {
            reason: RejectReason::UnknownStep,
            detail,
        } => FlowEngineError::NotFound(detail).into(),
        FlowError::Rejected {
            reason: RejectReason::InvalidChoice,
            detail,
        } => anyhow::anyhow!(detail),
        FlowError::Rejected { detail, .. } => FlowEngineError::Conflict {
            message: detail,
            existing: None,
        }
        .into(),
        other => anyhow::anyhow!(other.to_string()),
    }
}

/// What the HTTP layer drives. One run per ticket key.
pub struct FlowEngine {
    executor: Arc<Executor>,
    config: FlowEngineConfig,
    start_guard: Mutex<()>,
}

impl FlowEngine {
    /// Freezes flow transitions and joins all owned action workers before ownership is released.
    pub fn shutdown(&self) -> anyhow::Result<()> {
        let _admission = mutex_lock(&self.start_guard);
        self.executor.shutdown()
    }
    pub fn new(
        vault_root: PathBuf,
        state_dir: PathBuf,
        registry: Registry,
        ports: Ports,
        config: FlowEngineConfig,
    ) -> Arc<Self> {
        let store = FlowStore::new(vault_root, state_dir);
        Arc::new(FlowEngine {
            executor: Executor::new(store, registry, ports),
            config,
            start_guard: Mutex::new(()),
        })
    }

    pub fn executor(&self) -> &Arc<Executor> {
        &self.executor
    }

    fn load_definition(&self, name: &str) -> anyhow::Result<FlowDef> {
        aidp_core::safe_name::check(FLOW_LABEL, name)?;
        for dir in &self.config.flow_dirs {
            let file = dir.join(format!("{name}.{FLOW_EXTENSION}"));
            if file.is_file() {
                let text = std::fs::read_to_string(&file)?;
                return Ok(FlowDef::from_yaml(&text)?);
            }
        }
        Err(FlowEngineError::NotFound(format!("unknown flow `{name}`")).into())
    }

    /// Starts the run of `ticket` on the named flow, or the default one.
    /// Answers `Conflict` with the existing run when the ticket already has one.
    pub fn start(
        self: &Arc<Self>,
        ticket: Value,
        flow: Option<&str>,
        settings: Option<Value>,
    ) -> anyhow::Result<FlowState> {
        self.start_request(TicketFlowRequest {
            ticket,
            flow: flow.map(str::to_string),
            settings,
            ..TicketFlowRequest::default()
        })
    }

    /// Starts an execution, returning its persisted state when the same request is retried.
    pub fn start_request(
        self: &Arc<Self>,
        mut request: TicketFlowRequest,
    ) -> anyhow::Result<FlowState> {
        request
            .flow
            .get_or_insert_with(|| self.config.default_flow.clone());
        let _guard = mutex_lock(&self.start_guard);
        self.executor.require_running()?;
        if let Some(existing) = self.replayed_request(&request)? {
            return Ok(existing);
        }
        let def =
            self.load_definition(request.flow.as_deref().unwrap_or(&self.config.default_flow))?;
        self.start_request_locked(request, def)
    }

    fn replayed_request(&self, request: &TicketFlowRequest) -> anyhow::Result<Option<FlowState>> {
        let Some(key) = request.execution_key.as_deref() else {
            return Ok(None);
        };
        aidp_core::safe_name::check(crate::models::constants::flow_execution::EXECUTION_KEY, key)?;
        let Some(existing) = self.executor.store.index_get(key) else {
            return Ok(None);
        };
        let runtime = self.executor.store.load_runtime(&existing.repo, key);
        if runtime.start_request.as_ref() == Some(request) {
            let state = self.get(key)?;
            if state.is_fresh() {
                return self
                    .executor
                    .start_run(
                        &existing.repo,
                        state,
                        request.ticket.clone(),
                        request.entry.clone(),
                    )
                    .map(Some);
            }
            return Ok(Some(state));
        }
        Err(FlowEngineError::Conflict {
            message: format!("execution `{key}` already has different parameters"),
            existing: self.view(key).ok(),
        }
        .into())
    }

    /// Starts a run from an in-memory definition, validated like a file one.
    pub fn start_with_def(
        self: &Arc<Self>,
        ticket: Value,
        def: FlowDef,
        settings: Option<Value>,
    ) -> anyhow::Result<FlowState> {
        self.start_request_with_def(
            TicketFlowRequest {
                ticket,
                flow: Some(def.id.clone()),
                settings,
                ..TicketFlowRequest::default()
            },
            def,
        )
    }

    /// Starts a supplied definition with the same execution identity and policy as the API.
    pub fn start_request_with_def(
        self: &Arc<Self>,
        mut request: TicketFlowRequest,
        def: FlowDef,
    ) -> anyhow::Result<FlowState> {
        request.flow.get_or_insert_with(|| def.id.clone());
        let _guard = mutex_lock(&self.start_guard);
        self.executor.require_running()?;
        if let Some(existing) = self.replayed_request(&request)? {
            return Ok(existing);
        }
        self.start_request_locked(request, def)
    }

    fn start_request_locked(
        self: &Arc<Self>,
        request: TicketFlowRequest,
        def: FlowDef,
    ) -> anyhow::Result<FlowState> {
        crate::models::flow_execution_policy::validate_external_agents(request.policy.as_ref())?;
        crate::models::flow_execution_policy::validate_mcp_configuration(
            request
                .policy
                .as_ref()
                .and_then(|policy| policy.mcp_servers.as_deref()),
        )?;
        self.executor.ports.sessions.validate_mcp_reference(
            request
                .policy
                .as_ref()
                .and_then(|policy| policy.mcp_config_id.as_deref()),
        )?;
        let ticket_key = request
            .ticket
            .get(TICKET_KEY)
            .and_then(Value::as_str)
            .ok_or_else(|| anyhow::anyhow!(KEY_MISSING))?
            .to_string();
        aidp_core::safe_name::check(TICKET_KEY, &ticket_key)?;
        let key = request.execution_key.clone().unwrap_or(ticket_key);
        aidp_core::safe_name::check(
            crate::models::constants::flow_execution::EXECUTION_KEY,
            &key,
        )?;
        let catalog = self
            .executor
            .registry()
            .catalog(self.config.pipeline_templates.clone());
        let report = validate(&def, &catalog);
        if !report.is_valid() {
            let problems: Vec<&str> = report.errors.iter().map(|i| i.message.as_str()).collect();
            anyhow::bail!("invalid flow `{}`: {}", def.id, problems.join("; "));
        }
        let project = self
            .executor
            .ports
            .sessions
            .project_for(&request.ticket)
            .ok_or_else(|| FlowEngineError::Config(format!("no project for ticket `{key}`")))?;
        if let Some(existing) = self.executor.store.index_get(&key) {
            if is_active(existing.status) {
                let view = self.view(&key).ok();
                return Err(FlowEngineError::Conflict {
                    message: format!("ticket `{key}` already has an active flow run"),
                    existing: view,
                }
                .into());
            }
        }
        if let Some(entry) = request.entry.as_deref() {
            if !def.entries.contains_key(entry) {
                return Err(FlowEngineError::NotFound(format!(
                    "flow `{}` has no entry `{entry}`",
                    def.id
                ))
                .into());
            }
        }
        let mut state = FlowState::new(key, def);
        if let Some(settings) = request.settings.clone() {
            let Value::Object(map) = settings else {
                anyhow::bail!(SETTINGS_NOT_OBJECT);
            };
            state.settings.extend(map);
        }
        let mut runtime = self.executor.store.load_runtime(&project.repo, &state.key);
        runtime.start_request = Some(request.clone());
        self.executor
            .store
            .save_runtime(&project.repo, &state.key, &runtime)?;
        self.executor
            .start_run(&project.repo, state, request.ticket, request.entry)
    }

    /// The first of `flows` whose definition declares `entry`.
    pub fn flow_with_entry<'a>(&self, flows: &[&'a str], entry: &str) -> Option<&'a str> {
        flows.iter().copied().find(|name| {
            self.load_definition(name)
                .is_ok_and(|def| def.entries.contains_key(entry))
        })
    }

    pub fn get(&self, key: &str) -> anyhow::Result<FlowState> {
        self.executor.load(key).map(|(_, state)| state)
    }

    /// The run as the API serves it: the state, its repo and its open inbox entries.
    pub fn view(&self, key: &str) -> anyhow::Result<Value> {
        let (repo, state) = self.executor.load(key)?;
        let runtime = self.executor.store.load_runtime(&repo, key);
        let open: Vec<_> = runtime
            .inbox
            .iter()
            .filter(|item| item.resolved_at.is_none())
            .collect();
        let mut view = serde_json::to_value(&state)?;
        view[VIEW_REPO] = json!(repo);
        view[VIEW_INBOX] = json!(open);
        view[VIEW_SESSIONS] = json!(runtime.sessions);
        view[VIEW_PR] = json!(facts::pull_request(&state));
        Ok(view)
    }

    /// Runs filtered by run status and ticket service, most recently updated first.
    pub fn list(&self, status: Option<&str>, service: Option<&str>) -> Vec<IndexEntry> {
        let mut entries: Vec<IndexEntry> = self
            .executor
            .store
            .index()
            .into_values()
            .filter(|entry| status.is_none_or(|wanted| entry.status.as_wire() == wanted))
            .filter(|entry| service.is_none_or(|wanted| entry.service.as_deref() == Some(wanted)))
            .collect();
        entries.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        entries
    }

    /// True while a run still owns `key`: starting a new one would be refused.
    pub fn has_active_run(&self, key: &str) -> bool {
        self.executor
            .store
            .index_get(key)
            .is_some_and(|entry| is_active(entry.status))
    }

    /// What the live runs occupy: dev flows (worktrees) and, per project, the test flows that
    /// hold or wait for its main checkout.
    pub fn lane_load(&self) -> (usize, HashMap<String, usize>) {
        let (dev, _, test) = self.lane_load_with_bugs(&HashSet::new());
        (dev, test)
    }

    /// [`Self::lane_load`] with the dev flows of the `bug_keys` tickets counted apart: `(dev, bug, test)`.
    pub fn lane_load_with_bugs(
        &self,
        bug_keys: &HashSet<String>,
    ) -> (usize, usize, HashMap<String, usize>) {
        let mut dev = 0;
        let mut bug = 0;
        let mut test: HashMap<String, usize> = HashMap::new();
        for entry in self.executor.store.index().into_values() {
            if entry.status.is_terminal() {
                continue;
            }
            let Ok((_, state)) = self.executor.load(&entry.key) else {
                continue;
            };
            if state.def.workspace == Some(aidp_flows::models::enums::WorkspaceMode::Project) {
                if self.executor.project_checkout_held(&state) {
                    *test
                        .entry(entry.service.clone().unwrap_or_default())
                        .or_insert(0) += 1;
                }
            } else if !waits_on_merge(&state) {
                if bug_keys.contains(&entry.key) {
                    bug += 1;
                } else {
                    dev += 1;
                }
            }
        }
        (dev, bug, test)
    }

    pub fn journal(&self, key: &str) -> anyhow::Result<Vec<JournalLine>> {
        let (repo, _) = self.executor.load(key)?;
        Ok(self.executor.store.read_journal(&repo, key))
    }

    fn control(
        &self,
        key: &str,
        actor: &str,
        event: Event,
        line: String,
    ) -> anyhow::Result<FlowState> {
        let state = self.executor.deliver(key, event)?;
        self.executor.journal(key, Some(actor), line);
        Ok(state)
    }

    /// Lets the current step finish, then stops the run.
    pub fn pause(&self, key: &str, actor: &str) -> anyhow::Result<FlowState> {
        let at = self.executor.now();
        self.control(key, actor, Event::Pause { at }, format!("pause by {actor}"))
    }

    pub fn resume(&self, key: &str, actor: &str) -> anyhow::Result<FlowState> {
        let at = self.executor.now();
        self.control(
            key,
            actor,
            Event::Resume { at },
            format!("resume by {actor}"),
        )
    }

    /// Cancels the running session and ends the run; worktree and PR stay.
    pub fn abort(&self, key: &str, actor: &str) -> anyhow::Result<FlowState> {
        let at = self.executor.now();
        let state = self.control(key, actor, Event::Abort { at }, format!("abort by {actor}"))?;
        self.executor.cancel_manual(key);
        Ok(state)
    }

    pub fn retry(&self, key: &str, step: &str, actor: &str) -> anyhow::Result<FlowState> {
        let at = self.executor.now();
        let event = Event::Retry {
            step: step.to_string(),
            at,
        };
        self.control(key, actor, event, format!("retry {step} by {actor}"))
    }

    pub fn skip(&self, key: &str, step: &str, actor: &str) -> anyhow::Result<FlowState> {
        let at = self.executor.now();
        let event = Event::Skip {
            step: step.to_string(),
            at,
        };
        self.control(key, actor, event, format!("skip {step} by {actor}"))
    }

    /// Counts a failed step as passed, journaled under `actor`.
    pub fn mark_passed(&self, key: &str, step: &str, actor: &str) -> anyhow::Result<FlowState> {
        let at = self.executor.now();
        let event = Event::MarkPassed {
            step: step.to_string(),
            at,
        };
        self.control(key, actor, event, format!("mark {step} passed by {actor}"))
    }

    /// Records a human decision on a step waiting for one. Any other step answers `Conflict`.
    pub fn decide(
        &self,
        key: &str,
        step: &str,
        decision: HumanDecision,
        actor: &str,
        report: Option<String>,
    ) -> anyhow::Result<FlowState> {
        let event = Event::HumanDecision {
            step: step.to_string(),
            decision: decision.as_str().to_string(),
            actor: actor.to_string(),
            at: self.executor.now(),
            report,
        };
        self.executor.deliver(key, event)
    }

    pub fn approve(&self, key: &str, step: &str, actor: &str) -> anyhow::Result<FlowState> {
        self.decide(key, step, HumanDecision::Approve, actor, None)
    }

    pub fn reject(&self, key: &str, step: &str, actor: &str) -> anyhow::Result<FlowState> {
        self.decide(key, step, HumanDecision::Reject, actor, None)
    }

    pub fn click(&self, key: &str, step: &str, actor: &str) -> anyhow::Result<FlowState> {
        self.decide(key, step, HumanDecision::Click, actor, None)
    }

    /// A test verdict. The report text is stored as `flow/reports/<n>.md` and its path is the decision's report.
    pub fn verdict(
        &self,
        key: &str,
        step: &str,
        ok: bool,
        report: Option<&str>,
        actor: &str,
    ) -> anyhow::Result<FlowState> {
        let (repo, _) = self.executor.load(key)?;
        let stored = match report {
            Some(text) => Some(
                self.executor
                    .store
                    .write_report(&repo, key, text)?
                    .to_string_lossy()
                    .to_string(),
            ),
            None => None,
        };
        let decision = if ok {
            HumanDecision::Ok
        } else {
            HumanDecision::Ko
        };
        self.decide(key, step, decision, actor, stored)
    }

    /// Runs an action or a pipeline outside the flow's steps; journaled, and held to the one-writer rule.
    pub fn manual(
        &self,
        key: &str,
        request: ManualRequest,
        actor: &str,
    ) -> anyhow::Result<FlowState> {
        let at = self.executor.now();
        self.control(
            key,
            actor,
            Event::Manual { request, at },
            format!("manual request by {actor}"),
        )
    }

    /// The jobs run `key` hands to its caller, oldest first. Empty for a run that executes its own actions.
    pub fn external_jobs(&self, key: &str) -> anyhow::Result<Vec<ExternalJob>> {
        self.executor.external_jobs(key)
    }

    /// Applies the result the caller posted for job `job_id`: the run view, flagged when it replays an applied result.
    pub fn external_result(
        &self,
        key: &str,
        job_id: &str,
        result: ExternalResult,
        actor: &str,
    ) -> anyhow::Result<ExternalReply> {
        let duplicate = self
            .executor
            .apply_external_result(key, job_id, &result, actor)?;
        Ok(ExternalReply {
            view: self.view(key)?,
            duplicate,
        })
    }

    /// Open human decisions of every active run, as inbox entries of kind `flow.human`.
    pub fn inbox_entries(&self) -> Vec<InboxEntry> {
        let mut entries = Vec::new();
        for entry in self.executor.store.index().into_values() {
            if !is_active(entry.status) {
                continue;
            }
            let runtime = self.executor.store.load_runtime(&entry.repo, &entry.key);
            for item in runtime
                .inbox
                .into_iter()
                .filter(|item| item.resolved_at.is_none())
            {
                entries.push(InboxEntry {
                    kind: InboxKind::Human.as_str().to_string(),
                    action: InboxAction::Decide.as_str().to_string(),
                    title: item.prompt.clone().unwrap_or_else(|| item.step.clone()),
                    detail: Some(format!("{} / {}", entry.key, item.step)),
                    session_id: None,
                    fingerprint: Some(item.id),
                    automation_id: None,
                    project_id: None,
                    project_name: Some(entry.repo.clone()),
                    at: item.created_at,
                    urgency: INBOX_URGENCY,
                });
            }
        }
        entries
    }
}

/// A flow parked on its merge approval has finished its work: it holds no dev slot.
pub(crate) fn waits_on_merge(state: &aidp_flows::FlowState) -> bool {
    use crate::models::constants::plan_slots::MERGE_PHASE;
    state.status == aidp_flows::FlowStatus::WaitingHuman
        && state
            .current
            .as_ref()
            .and_then(|step| state.steps.get(step))
            .and_then(|step| step.phase.as_deref())
            .is_some_and(|phase| phase.eq_ignore_ascii_case(MERGE_PHASE))
}
