//! Automations: cron, interval, webhook and event triggers that start sessions.
//! Guards: 5-hour rate-limit window utilization and anti-loop on own events.

use crate::bus::Bus;
use crate::ids::new_id;
use crate::launcher::RunLauncher;
use crate::models::launched_run::LaunchedRun;
use crate::native_runs::dispatch::autonomous_env;
use crate::projects::Projects;
use crate::sessions::{
    Launch, Origin, SessionManager, SessionRecord, StartRequest, EVENT_SESSION_ENDED,
};
use crate::store::{append_jsonl, read_json, read_jsonl, write_json};
use crate::sync::lock;
use crate::template::{render, render_map};
use crate::time::now_iso;
use aidp_core::clock::{Clock, SystemClock};
use anyhow::{anyhow, bail};
use chrono::{DateTime, Duration, Local, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration as StdDuration;
use tracing::{error, info};

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Trigger {
    Cron {
        expr: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timezone: Option<String>,
    },
    Every {
        minutes: f64,
    },
    At {
        iso: String,
    },
    Webhook,
    Event {
        on: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        filter: Option<BTreeMap<String, String>>,
    },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Action {
    Launch {
        project_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        engine: Option<String>,
        template_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        inputs: Option<BTreeMap<String, String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        permission_mode: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        effort: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_budget_usd: Option<f64>,
        /// Skip launching when the 5h window is at/above this ratio (0-1). Default AIDP_MAX_UTILIZATION or 0.9.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_utilization: Option<f64>,
        /// Gateway session id whose vault-run artefacts feed the launched session's prompt.
        /// Rendered like the other fields; not offered on `Action::Prompt`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        context_from: Option<String>,
        /// Auto-approve every gate of the native run this launch starts.
        #[serde(default)]
        autonomous: bool,
    },
    Prompt {
        project_id: String,
        prompt: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        permission_mode: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_budget_usd: Option<f64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_utilization: Option<f64>,
    },
    Webhook {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        method: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        body: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        headers: Option<BTreeMap<String, String>>,
    },
    #[serde(rename = "none")]
    NoOp,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Delivery {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Fire on these outcomes only (default all): done, failed, cancelled, skipped, question.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on: Option<Vec<String>>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Automation {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub enabled: bool,
    pub trigger: Trigger,
    pub action: Action,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery: Option<Delivery>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_run_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_status: Option<String>,
    /// Computed, never persisted.
    #[serde(default)]
    pub next_run_at: Option<String>,
    #[serde(default)]
    pub run_count: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct AutomationRun {
    pub id: String,
    pub automation_id: String,
    pub automation_name: String,
    pub reason: String,
    pub started_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<String>,
    /// started | done | failed | cancelled | skipped
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trace: Vec<TraceStep>,
}

/// One step of the automation trace: trigger, filter, guardrail, action, delivery.
/// The same code builds it for a real run and for the unrecorded dry-run.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct TraceStep {
    /// trigger | filter | guardrail | action | delivery
    pub step: String,
    /// matched | rejected | passed | blocked | rendered | planned | skipped | pending | na
    pub outcome: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub data: Value,
    pub at: String,
}

/// The launch of an automation: `base` stamped with its origin, ids and the autonomous switch.
fn automation_start_request(
    base: StartRequest,
    reason: &str,
    automation: &Automation,
    run_id: &str,
) -> StartRequest {
    StartRequest {
        origin: Some(Origin {
            kind: reason.to_string(),
            r#ref: Some(automation.id.clone()),
            label: Some(automation.name.clone()),
        }),
        automation_id: Some(automation.id.clone()),
        automation_run_id: Some(run_id.to_string()),
        env: autonomous_env(),
        ..base
    }
}

fn trace_step(step: &str, outcome: &str, detail: Option<String>, data: Value) -> TraceStep {
    TraceStep {
        step: step.to_string(),
        outcome: outcome.to_string(),
        detail,
        data,
        at: now_iso(),
    }
}

/// The action/rendered step shared by the real run and the dry-run.
/// It logs header names only, never a header value, since values can carry a secret.
fn render_action_trace(action: &Action, rctx: &Value) -> TraceStep {
    match action {
        Action::Launch {
            project_id,
            template_id,
            target,
            inputs,
            ..
        } => {
            let pid = render(project_id, rctx);
            let data = json!({
                "projectId": if pid.is_empty() { project_id.clone() } else { pid },
                "templateId": render(template_id, rctx),
                "target": target.as_ref().map(|t| render(t, rctx)),
                "inputs": render_map(inputs, rctx),
            });
            trace_step(STEP_ACTION, OUTCOME_RENDERED, None, data)
        }
        Action::Prompt { prompt, .. } => {
            let rendered = render(prompt, rctx);
            trace_step(
                STEP_ACTION,
                OUTCOME_RENDERED,
                None,
                json!({ "prompt": head(&rendered, 300) }),
            )
        }
        Action::Webhook {
            url,
            method,
            headers,
            ..
        } => {
            let header_names: Vec<String> = headers
                .as_ref()
                .map(|h| h.keys().cloned().collect())
                .unwrap_or_default();
            let data = json!({
                "url": aidp_integrations::redact_url(&render(url, rctx)),
                "method": method.clone().unwrap_or_else(|| METHOD_POST.to_string()),
                "headerNames": header_names,
            });
            trace_step(STEP_ACTION, OUTCOME_RENDERED, None, data)
        }
        Action::NoOp => trace_step(STEP_ACTION, OUTCOME_RENDERED, None, Value::Null),
    }
}

/// The delivery step (planned or skipped), mirroring the early returns of `deliver`.
/// `hypothetical_detail` set marks the delivery as not actually sent (dry-run).
fn delivery_trace(
    delivery: &Option<Delivery>,
    outcome: &str,
    hypothetical_detail: Option<&str>,
) -> TraceStep {
    let Some(d) = delivery else {
        return trace_step(
            STEP_DELIVERY,
            OUTCOME_SKIPPED,
            Some("no delivery configured".to_string()),
            Value::Null,
        );
    };
    if d.kind != DELIVERY_KIND_WEBHOOK {
        return trace_step(
            STEP_DELIVERY,
            OUTCOME_SKIPPED,
            Some(format!("delivery kind \"{}\" not applicable", d.kind)),
            Value::Null,
        );
    }
    let has_url = d.url.as_deref().map(|u| !u.is_empty()).unwrap_or(false);
    if !has_url {
        return trace_step(
            STEP_DELIVERY,
            OUTCOME_SKIPPED,
            Some("no delivery url configured".to_string()),
            Value::Null,
        );
    }
    if let Some(on) = &d.on {
        if !on.iter().any(|o| o == outcome) {
            return trace_step(
                STEP_DELIVERY,
                OUTCOME_SKIPPED,
                Some(format!("outcome \"{outcome}\" not in delivery.on")),
                Value::Null,
            );
        }
    }
    trace_step(
        STEP_DELIVERY,
        OUTCOME_PLANNED,
        hypothetical_detail.map(|s| s.to_string()),
        json!({ "url": d.url.as_deref().map(aidp_integrations::redact_url), "outcome": outcome }),
    )
}

/// Sleep up to `total` duration, waking every ~200ms to check the shutdown flag.
/// Returns false if interrupted by the flag, true if the full duration elapsed.
fn sleep_unless_stopped(shutdown: &crate::shutdown::Shutdown, total: StdDuration) -> bool {
    let mut elapsed = StdDuration::ZERO;
    let slice = StdDuration::from_millis(200);
    while elapsed < total {
        if shutdown.is_set() {
            return false;
        }
        let remaining = total - elapsed;
        let to_sleep = std::cmp::min(remaining, slice);
        thread::sleep(to_sleep);
        elapsed += to_sleep;
    }
    true
}

/// State file holding the persisted automations.
const AUTOMATIONS_FILE: &str = "automations.json";
/// Append-only file of automation runs.
const RUNS_FILE: &str = "automation-runs.jsonl";
/// Engine of a prompt action.
const ENGINE_PROMPT: &str = "prompt";
/// Default permission mode of a launched session.
const PERMISSION_ACCEPT_EDITS: &str = "acceptEdits";
/// Delivery text label for a session started from a bare prompt.
const TEMPLATE_LABEL_PROMPT: &str = "prompt";
/// HTTP method used by default for a webhook action.
const METHOD_POST: &str = "POST";
/// HTTP PUT method, upper case.
const METHOD_PUT_UPPER: &str = "PUT";
/// HTTP PUT method, lower case.
const METHOD_PUT_LOWER: &str = "put";
/// Content-Type header name.
const HEADER_CONTENT_TYPE: &str = "Content-Type";
/// JSON media type.
const CONTENT_TYPE_JSON: &str = "application/json";
/// The only delivery kind that sends anything.
const DELIVERY_KIND_WEBHOOK: &str = "webhook";
/// Event pattern matching every event type.
const PATTERN_ALL: &str = "*";
/// Suffix of an event pattern matching a type prefix.
const PATTERN_PREFIX_SUFFIX: &str = ".*";
/// Prefix of every automation bus event, ignored by event triggers.
const AUTOMATION_EVENT_PREFIX: &str = "automation.";
/// Prefix of the bus events mirroring GitHub webhook deliveries.
const GITHUB_EVENT_PREFIX: &str = "github.";
/// Bus event emitted when an automation fails outside a run.
const EVENT_AUTOMATION_ERROR: &str = "automation.error";
/// Bus event emitted when an automation is saved.
const EVENT_AUTOMATION_SAVED: &str = "automation.saved";
/// Bus event emitted for each automation run.
const EVENT_AUTOMATION_RUN: &str = "automation.run";
/// Bus event emitted when a webhook automation is triggered.
const EVENT_WEBHOOK_RECEIVED: &str = "webhook.received";
/// Bus event emitted when a session asks a question.
const EVENT_SESSION_QUESTION: &str = "session.question";
/// Run reason of an event-triggered run.
const REASON_EVENT: &str = "event";
/// Run reason of a webhook-triggered run.
const REASON_WEBHOOK: &str = "webhook";
/// Trigger reason recorded by a dry-run.
const REASON_TEST: &str = "test";
/// Trace step: trigger.
const STEP_TRIGGER: &str = "trigger";
/// Trace step: filter.
const STEP_FILTER: &str = "filter";
/// Trace step: guardrail.
const STEP_GUARDRAIL: &str = "guardrail";
/// Trace step: action.
const STEP_ACTION: &str = "action";
/// Trace step: delivery.
const STEP_DELIVERY: &str = "delivery";
/// Trace outcome: matched.
const OUTCOME_MATCHED: &str = "matched";
/// Trace outcome: rejected.
const OUTCOME_REJECTED: &str = "rejected";
/// Trace outcome: passed.
const OUTCOME_PASSED: &str = "passed";
/// Trace outcome: blocked.
const OUTCOME_BLOCKED: &str = "blocked";
/// Trace outcome: rendered.
const OUTCOME_RENDERED: &str = "rendered";
/// Trace outcome: planned.
const OUTCOME_PLANNED: &str = "planned";
/// Trace outcome and run status: skipped.
const OUTCOME_SKIPPED: &str = "skipped";
/// Trace outcome: pending.
const OUTCOME_PENDING: &str = "pending";
/// Trace outcome: not applicable.
const OUTCOME_NOT_APPLICABLE: &str = "na";
/// Delivery outcome: the session asked a question.
const OUTCOME_QUESTION: &str = "question";
/// Run status: done.
const RUN_STATUS_DONE: &str = "done";
/// Run status: failed.
const RUN_STATUS_FAILED: &str = "failed";
/// Run status: cancelled.
const RUN_STATUS_CANCELLED: &str = "cancelled";
/// Dry-run status: the automation would run.
const DRY_RUN_WOULD_RUN: &str = "would-run";
/// Dry-run status: the trigger filter rejected the event.
const DRY_RUN_FILTERED: &str = "filtered";

/// Bus event emitted when a due automation is not launched.
const AUTOMATION_SKIPPED_EVENT: &str = "automation.skipped";
/// Skip reason: the previous run of the same automation has not finished.
const SKIPPED_ALREADY_RUNNING: &str = "already running";
/// Trigger label of a scheduled (cron or interval) automation run.
const TRIGGER_CRON: &str = "cron";
/// `AutomationRun.status` while a Launch/Prompt session has started or is
/// queued and has not yet settled (`on_bus_event(session.ended)` recomputes it).
const RUN_STATUS_STARTED: &str = "started";
/// Bus event a queued launch resolves to (started, cancelled or failed).
const SESSION_DEQUEUED_EVENT: &str = "session.dequeued";
/// `session.dequeued` reason: the queued entry was withdrawn before starting.
const DEQUEUE_REASON_CANCELLED: &str = "cancelled";
/// `session.dequeued` reason: the queued entry failed to start.
const DEQUEUE_REASON_FAILED: &str = "failed";

/// Marks one automation as running; dropping it releases the automation.
/// After `keep_until_settled`, `release_pending` owns the release instead.
pub struct InFlightGuard {
    engine: Arc<AutomationEngine>,
    id: Option<String>,
}

impl InFlightGuard {
    /// Keeps the in-flight mark until `run.id` settles instead of releasing on drop.
    /// Disarms `Drop` so the mark is not removed twice.
    fn keep_until_settled(mut self, run: &AutomationRun) {
        let Some(automation_id) = self.id.take() else {
            return;
        };
        lock(&self.engine.pending_runs).insert(
            run.id.clone(),
            PendingRun {
                automation_id,
                queue_id: run.queue_id.clone(),
            },
        );
    }
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        if let Some(id) = self.id.take() {
            lock(&self.engine.in_flight).remove(&id);
        }
    }
}

/// A Launch/Prompt run whose in-flight mark outlives its guard, keyed by run id.
/// Holds no guard and no `Arc`, so it creates no reference cycle.
struct PendingRun {
    automation_id: String,
    queue_id: Option<String>,
}

pub struct AutomationEngine {
    file: PathBuf,
    runs_file: PathBuf,
    list: Mutex<Vec<Automation>>,
    runs: Mutex<HashMap<String, AutomationRun>>,
    next: Mutex<HashMap<String, DateTime<Utc>>>,
    in_flight: Mutex<HashSet<String>>,
    pending_runs: Mutex<HashMap<String, PendingRun>>,
    sessions: Arc<SessionManager>,
    projects: Arc<Projects>,
    bus: Arc<Bus>,
    launcher: Mutex<Option<Arc<RunLauncher>>>,
}

fn pattern_matches(pattern: &str, ty: &str) -> bool {
    pattern == PATTERN_ALL
        || pattern == ty
        || (pattern.ends_with(PATTERN_PREFIX_SUFFIX)
            && ty.starts_with(&pattern[..pattern.len() - 1]))
}

fn value_str(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Whether an `Event` trigger fires for `event_type`/`data`; `Err` names the failing part.
/// Shared by the real dispatch path and the dry-run; a non-`Event` trigger never matches.
pub fn event_trigger_matches(
    trigger: &Trigger,
    event_type: &str,
    data: &Value,
) -> Result<(), String> {
    let Trigger::Event { on, filter } = trigger else {
        return Err("not an event trigger".to_string());
    };
    if !on.iter().any(|p| pattern_matches(p, event_type)) {
        return Err(format!("event \"{event_type}\" matches none of {on:?}"));
    }
    if let Some(f) = filter {
        for (k, v) in f {
            let actual = value_str(&data[k.as_str()]);
            if actual != *v {
                return Err(format!(
                    "filter key \"{k}\" expected \"{v}\" got \"{actual}\""
                ));
            }
        }
    }
    Ok(())
}

fn head(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// When a trigger fires next, counted from `from`.
fn next_for(t: &Trigger, from: DateTime<Utc>) -> Option<DateTime<Utc>> {
    match t {
        Trigger::Cron { expr, .. } => crate::cron::parse(expr)
            .ok()?
            .next_after(from.with_timezone(&Local))
            .map(|d| d.with_timezone(&Utc)),
        Trigger::Every { minutes } => {
            Some(from + Duration::seconds((minutes.max(1.0) * 60.0) as i64))
        }
        Trigger::At { iso } => {
            let t = DateTime::parse_from_rfc3339(iso).ok()?.with_timezone(&Utc);
            if t > from {
                Some(t)
            } else {
                None
            }
        }
        Trigger::Webhook | Trigger::Event { .. } => None,
    }
}

impl AutomationEngine {
    /// Routes launches through the native-by-default launcher; without it they stay sessions.
    pub fn set_launcher(&self, launcher: Arc<RunLauncher>) {
        *lock(&self.launcher) = Some(launcher);
    }

    fn launch(&self, request: StartRequest, autonomous: bool) -> anyhow::Result<LaunchedRun> {
        let launcher = lock(&self.launcher).clone();
        match launcher {
            Some(launcher) => launcher.launch(request, autonomous),
            None => Ok(LaunchedRun::Session(self.sessions.start(request)?)),
        }
    }

    /// True when the run holds a session or queue slot that ends with a session event.
    fn settles_with_session(&self, run: &AutomationRun) -> bool {
        run.queue_id.is_some()
            || run
                .session_id
                .as_deref()
                .is_some_and(|id| self.sessions.get(id).is_some())
    }

    pub fn load(
        state_dir: &Path,
        sessions: Arc<SessionManager>,
        projects: Arc<Projects>,
        bus: Arc<Bus>,
    ) -> Arc<Self> {
        let file = state_dir.join(AUTOMATIONS_FILE);
        let runs_file = state_dir.join(RUNS_FILE);
        let list: Vec<Automation> = read_json(&file, Vec::new());
        let engine = Arc::new(AutomationEngine {
            file,
            runs_file,
            list: Mutex::new(list),
            runs: Mutex::new(HashMap::new()),
            next: Mutex::new(HashMap::new()),
            in_flight: Mutex::new(HashSet::new()),
            pending_runs: Mutex::new(HashMap::new()),
            sessions,
            projects,
            bus,
            launcher: Mutex::new(None),
        });
        for a in engine.list_raw() {
            engine.reschedule(&a);
        }
        info!(
            "{} automation(s), {} scheduled",
            lock(&engine.list).len(),
            lock(&engine.next).len()
        );
        engine
    }

    /// Scheduler tick + event hooks. Call once after `load`.
    pub fn start_threads(self: &Arc<Self>, shutdown: Arc<crate::shutdown::Shutdown>) {
        let me = self.clone();
        let shutdown_tick = shutdown.clone();
        thread::spawn(move || {
            let _loop_guard = crate::threads::enter_loop(crate::threads::AUTOMATIONS_SCHEDULER);
            loop {
                if shutdown_tick.is_set() {
                    break;
                }
                if !sleep_unless_stopped(&shutdown_tick, StdDuration::from_secs(5)) {
                    break;
                }
                crate::threads::guarded(crate::threads::AUTOMATIONS_SCHEDULER, || {
                    me.scheduler_pass()
                });
            }
        });

        let me = self.clone();
        let shutdown_bus = shutdown.clone();
        let rx = self.bus.subscribe();
        thread::spawn(move || {
            let _loop_guard = crate::threads::enter_loop(crate::threads::AUTOMATIONS_BUS);
            for ev in rx.iter() {
                if shutdown_bus.is_set() {
                    break;
                }
                crate::threads::guarded(crate::threads::AUTOMATIONS_BUS, || me.on_bus_event(&ev));
            }
        });
    }

    /// One scheduler pass: reschedules every due automation, then launches it.
    fn scheduler_pass(self: &Arc<Self>) {
        let now = SystemClock.now_utc();
        let due: Vec<String> = lock(&self.next)
            .iter()
            .filter(|(_, t)| **t <= now)
            .map(|(id, _)| id.clone())
            .collect();
        for id in due {
            let Some(a) = self.get(&id) else {
                lock(&self.next).remove(&id);
                continue;
            };
            {
                match next_for(&a.trigger, now) {
                    Some(t) => {
                        lock(&self.next).insert(id.clone(), t);
                    }
                    None => {
                        lock(&self.next).remove(&id);
                    }
                }
            }
            self.launch_due(a);
        }
    }

    /// Claims `id` for one run; `None` when a run of that automation is still in flight.
    fn try_mark_in_flight(self: &Arc<Self>, id: &str) -> Option<InFlightGuard> {
        if !lock(&self.in_flight).insert(id.to_string()) {
            return None;
        }
        Some(InFlightGuard {
            engine: self.clone(),
            id: Some(id.to_string()),
        })
    }

    /// Removes every `pending_runs` entry `matches` accepts (given the run id
    /// it is keyed by), and releases each one's automation from `in_flight`.
    fn release_pending<F: Fn(&str, &PendingRun) -> bool>(&self, matches: F) {
        let mut pending = lock(&self.pending_runs);
        let settled: Vec<String> = pending
            .iter()
            .filter(|(run_id, p)| matches(run_id, p))
            .map(|(run_id, _)| run_id.clone())
            .collect();
        for run_id in settled {
            if let Some(p) = pending.remove(&run_id) {
                lock(&self.in_flight).remove(&p.automation_id);
            }
        }
    }

    /// Frees the in-flight mark of a queued run that never started, matched by queue id.
    /// Only `cancelled` and `failed` release it; `started` means the entry ran.
    fn release_pending_queue(&self, data: &Value) {
        let reason = data["reason"].as_str().unwrap_or("");
        if !matches!(reason, DEQUEUE_REASON_CANCELLED | DEQUEUE_REASON_FAILED) {
            return;
        }
        let Some(queue_id) = data["queueId"].as_str() else {
            return;
        };
        self.release_pending(|_, p| p.queue_id.as_deref() == Some(queue_id));
    }

    /// Runs a due automation on its own thread, or emits `automation.skipped`
    /// when its previous run has not finished yet.
    fn launch_due(self: &Arc<Self>, a: Automation) {
        let trigger = TRIGGER_CRON;
        let Some(guard) = self.try_mark_in_flight(&a.id) else {
            self.bus.emit(
                AUTOMATION_SKIPPED_EVENT,
                json!({
                    "automationId": a.id,
                    "name": a.name,
                    "trigger": trigger,
                    "skipped": SKIPPED_ALREADY_RUNNING,
                }),
            );
            return;
        };
        let me = self.clone();
        thread::spawn(
            move || match me.run_automation(&a.id, TRIGGER_CRON, json!({})) {
                Ok(run) if run.status == RUN_STATUS_STARTED && me.settles_with_session(&run) => {
                    guard.keep_until_settled(&run);
                }
                Ok(_) => drop(guard),
                Err(e) => {
                    drop(guard);
                    me.bus.emit(
                        EVENT_AUTOMATION_ERROR,
                        json!({ "automationId": a.id, "name": a.name, "error": e.to_string() }),
                    );
                }
            },
        );
    }

    fn save_list(&self, list: &[Automation]) {
        if let Err(e) = write_json(&self.file, &list) {
            error!("cannot save automations: {e}");
        }
    }

    fn list_raw(&self) -> Vec<Automation> {
        lock(&self.list).clone()
    }

    pub fn list(&self) -> Vec<Automation> {
        let next = lock(&self.next);
        self.list_raw()
            .into_iter()
            .map(|mut a| {
                a.next_run_at = next
                    .get(&a.id)
                    .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
                a
            })
            .collect()
    }

    pub fn get(&self, id: &str) -> Option<Automation> {
        lock(&self.list).iter().find(|a| a.id == id).cloned()
    }

    fn reschedule(&self, a: &Automation) {
        let mut n = lock(&self.next);
        if !a.enabled {
            n.remove(&a.id);
            return;
        }
        match next_for(&a.trigger, SystemClock.now_utc()) {
            Some(t) => {
                n.insert(a.id.clone(), t);
            }
            None => {
                n.remove(&a.id);
            }
        }
    }

    /// Create or update from a JSON document (`id` present ⇒ update).
    pub fn upsert(&self, input: Value) -> anyhow::Result<Automation> {
        let id = input["id"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string());
        let existing = id.as_deref().and_then(|i| self.get(i));
        let name = input["name"]
            .as_str()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| anyhow!("name is required"))?
            .to_string();
        let trigger: Trigger = serde_json::from_value(input["trigger"].clone())
            .map_err(|e| anyhow!("invalid trigger: {e}"))?;
        let action: Action = serde_json::from_value(input["action"].clone())
            .map_err(|e| anyhow!("invalid action: {e}"))?;
        let delivery: Option<Delivery> = match input.get("delivery") {
            None | Some(Value::Null) => None,
            Some(v) => Some(
                serde_json::from_value(v.clone()).map_err(|e| anyhow!("invalid delivery: {e}"))?,
            ),
        };
        if let Trigger::Cron { expr, .. } = &trigger {
            crate::cron::parse(expr).map_err(|e| anyhow!("invalid cron expression: {e}"))?;
        }
        if let Trigger::At { iso } = &trigger {
            DateTime::parse_from_rfc3339(iso).map_err(|e| anyhow!("invalid date: {e}"))?;
        }
        let now = now_iso();
        let a = Automation {
            id: existing
                .as_ref()
                .map(|e| e.id.clone())
                .or(id)
                .unwrap_or_else(new_id),
            name,
            description: input["description"]
                .as_str()
                .map(|s| s.to_string())
                .filter(|s| !s.is_empty()),
            enabled: input["enabled"]
                .as_bool()
                .or(existing.as_ref().map(|e| e.enabled))
                .unwrap_or(true),
            trigger,
            action,
            delivery,
            created_at: existing
                .as_ref()
                .map(|e| e.created_at.clone())
                .unwrap_or_else(|| now.clone()),
            updated_at: now,
            last_run_at: existing.as_ref().and_then(|e| e.last_run_at.clone()),
            last_status: existing.as_ref().and_then(|e| e.last_status.clone()),
            next_run_at: None,
            run_count: existing.as_ref().map(|e| e.run_count).unwrap_or(0),
        };
        {
            let mut l = lock(&self.list);
            if let Some(pos) = l.iter().position(|x| x.id == a.id) {
                l[pos] = a.clone();
            } else {
                l.push(a.clone());
            }
            self.save_list(&l);
        }
        self.reschedule(&a);
        self.bus.emit(
            EVENT_AUTOMATION_SAVED,
            json!({ "automationId": a.id, "name": a.name, "enabled": a.enabled }),
        );
        Ok(a)
    }

    pub fn delete(&self, id: &str) -> bool {
        let mut l = lock(&self.list);
        let before = l.len();
        l.retain(|a| a.id != id);
        lock(&self.next).remove(id);
        if l.len() == before {
            return false;
        }
        self.save_list(&l);
        true
    }

    pub fn list_runs(&self, automation_id: Option<&str>, limit: usize) -> Vec<AutomationRun> {
        let mut by_id: HashMap<String, AutomationRun> = HashMap::new();
        for r in read_jsonl::<AutomationRun>(&self.runs_file) {
            by_id.insert(r.id.clone(), r);
        }
        let mut runs: Vec<AutomationRun> = by_id
            .into_values()
            .filter(|r| automation_id.map(|a| r.automation_id == a).unwrap_or(true))
            .collect();
        runs.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        runs.truncate(if limit == 0 { 50 } else { limit });
        runs
    }

    fn record_run(&self, run: &AutomationRun) {
        lock(&self.runs).insert(run.id.clone(), run.clone());
        append_jsonl(&self.runs_file, run);
        let mut l = lock(&self.list);
        if let Some(a) = l.iter_mut().find(|a| a.id == run.automation_id) {
            a.last_run_at = Some(run.started_at.clone());
            a.last_status = Some(run.status.clone());
        }
        self.save_list(&l);
    }

    /// The 5-hour window is the real budget on a Max subscription.
    fn utilization_block(&self, limit: Option<f64>) -> Option<String> {
        let default_max = std::env::var("AIDP_MAX_UTILIZATION")
            .ok()
            .and_then(|v| v.parse::<f64>().ok())
            .unwrap_or(0.9);
        let max = limit.unwrap_or(default_max).clamp(0.0, 1.0);
        let rl = self.sessions.rate_limit()?;
        let w = rl.five_hour?;
        if w.utilization < max {
            return None;
        }
        let reset = DateTime::<Utc>::from_timestamp(w.resets_at, 0)
            .map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
            .unwrap_or_else(|| "?".into());
        Some(format!(
            "5h window at {}% ≥ {}% (resets {reset})",
            (w.utilization * 100.0).round(),
            (max * 100.0).round()
        ))
    }

    pub fn run_automation(
        &self,
        id: &str,
        reason: &str,
        ctx: Value,
    ) -> anyhow::Result<AutomationRun> {
        let a = self
            .get(id)
            .ok_or_else(|| anyhow!("automation not found"))?;
        let mut run = AutomationRun {
            id: new_id(),
            automation_id: a.id.clone(),
            automation_name: a.name.clone(),
            reason: reason.to_string(),
            started_at: now_iso(),
            ended_at: None,
            status: RUN_STATUS_STARTED.to_string(),
            session_id: None,
            queue_id: None,
            error: None,
            note: None,
            trace: Vec::new(),
        };
        {
            let mut l = lock(&self.list);
            if let Some(x) = l.iter_mut().find(|x| x.id == a.id) {
                x.run_count += 1;
            }
        }
        let mut rctx = if ctx.is_object() { ctx } else { json!({}) };
        rctx["automation"] = json!({ "id": a.id, "name": a.name });
        rctx["now"] = json!(run.started_at);

        let mut trace: Vec<TraceStep> = vec![trace_step(
            STEP_TRIGGER,
            OUTCOME_MATCHED,
            None,
            json!({ "reason": reason }),
        )];

        let outcome: anyhow::Result<()> = (|| match &a.action {
            Action::Launch {
                max_utilization, ..
            }
            | Action::Prompt {
                max_utilization, ..
            } => {
                if let Some(why) = self.utilization_block(*max_utilization) {
                    run.status = OUTCOME_SKIPPED.to_string();
                    run.note = Some(why.clone());
                    run.ended_at = Some(now_iso());
                    trace.push(trace_step(
                        STEP_GUARDRAIL,
                        OUTCOME_BLOCKED,
                        Some(why),
                        Value::Null,
                    ));
                    return Ok(());
                }
                trace.push(trace_step(
                    STEP_GUARDRAIL,
                    OUTCOME_PASSED,
                    None,
                    Value::Null,
                ));
                trace.push(render_action_trace(&a.action, &rctx));
                let mut autonomous = false;
                let base = match &a.action {
                    Action::Launch {
                        autonomous: launch_autonomous,
                        project_id,
                        engine,
                        template_id,
                        target,
                        inputs,
                        permission_mode,
                        model,
                        effort,
                        max_budget_usd,
                        context_from,
                        ..
                    } => {
                        let pid = render(project_id, &rctx);
                        autonomous = *launch_autonomous;
                        StartRequest {
                            project_id: if pid.is_empty() {
                                project_id.clone()
                            } else {
                                pid
                            },
                            engine: engine.clone(),
                            template_id: Some(render(template_id, &rctx)),
                            target: target.as_ref().map(|t| render(t, &rctx)),
                            inputs: render_map(inputs, &rctx),
                            permission_mode: Some(
                                permission_mode
                                    .clone()
                                    .unwrap_or_else(|| PERMISSION_ACCEPT_EDITS.to_string()),
                            ),
                            model: model.clone(),
                            effort: effort.clone(),
                            max_budget_usd: *max_budget_usd,
                            context_from: context_from
                                .as_ref()
                                .map(|c| render(c, &rctx))
                                .filter(|c| !c.trim().is_empty()),
                            ..Default::default()
                        }
                    }
                    Action::Prompt {
                        project_id,
                        prompt,
                        permission_mode,
                        model,
                        max_budget_usd,
                        ..
                    } => StartRequest {
                        project_id: project_id.clone(),
                        engine: Some(ENGINE_PROMPT.to_string()),
                        prompt: Some(render(prompt, &rctx)),
                        permission_mode: Some(
                            permission_mode
                                .clone()
                                .unwrap_or_else(|| PERMISSION_ACCEPT_EDITS.to_string()),
                        ),
                        model: model.clone(),
                        max_budget_usd: *max_budget_usd,
                        ..Default::default()
                    },
                    _ => unreachable!(),
                };
                let req = automation_start_request(base, reason, &a, &run.id);
                let launch = self.launch(req, autonomous)?;
                match launch {
                    LaunchedRun::Native { run_id } => {
                        run.session_id = Some(run_id);
                    }
                    LaunchedRun::Session(Launch::Started(rec)) => {
                        run.session_id = Some(rec.id);
                    }
                    LaunchedRun::Session(Launch::Queued(q)) => {
                        run.queue_id = Some(q.id.clone());
                        run.note =
                            Some("queued behind a running session on this project".to_string());
                    }
                }
                Ok(())
            }
            Action::Webhook {
                url,
                method,
                body,
                headers,
            } => {
                trace.push(trace_step(
                    STEP_GUARDRAIL,
                    OUTCOME_NOT_APPLICABLE,
                    Some("not applicable: this action starts no session".to_string()),
                    Value::Null,
                ));
                trace.push(render_action_trace(&a.action, &rctx));
                let u = render(url, &rctx);
                let mut r = match method.as_deref() {
                    Some(METHOD_PUT_UPPER) | Some(METHOD_PUT_LOWER) => crate::net::agent().put(&u),
                    _ => crate::net::agent().post(&u),
                };
                r = r.set(HEADER_CONTENT_TYPE, CONTENT_TYPE_JSON);
                if let Some(h) = headers {
                    for (k, v) in h {
                        r = r.set(k, v);
                    }
                }
                let payload = body
                    .as_ref()
                    .map(|b| render(b, &rctx))
                    .unwrap_or_else(|| rctx.to_string());
                match r.send_string(&payload) {
                    Ok(resp) => {
                        run.status = RUN_STATUS_DONE.to_string();
                        run.note = Some(format!("HTTP {}", resp.status()));
                    }
                    Err(ureq::Error::Status(code, _)) => {
                        run.status = RUN_STATUS_FAILED.to_string();
                        run.note = Some(format!("HTTP {code}"));
                    }
                    Err(e) => {
                        run.status = RUN_STATUS_FAILED.to_string();
                        run.error = Some(aidp_integrations::redact_url_in(&e.to_string(), &u));
                    }
                }
                run.ended_at = Some(now_iso());
                Ok(())
            }
            Action::NoOp => {
                trace.push(trace_step(
                    STEP_GUARDRAIL,
                    OUTCOME_NOT_APPLICABLE,
                    Some("not applicable: this action starts no session".to_string()),
                    Value::Null,
                ));
                trace.push(render_action_trace(&a.action, &rctx));
                run.status = RUN_STATUS_DONE.to_string();
                run.ended_at = Some(now_iso());
                Ok(())
            }
        })();
        if let Err(e) = outcome {
            run.status = RUN_STATUS_FAILED.to_string();
            run.error = Some(e.to_string());
            run.ended_at = Some(now_iso());
        }
        if run.status == RUN_STATUS_STARTED {
            trace.push(trace_step(
                STEP_DELIVERY,
                OUTCOME_PENDING,
                Some(
                    "outcome not yet settled; delivery re-evaluated when the session ends"
                        .to_string(),
                ),
                Value::Null,
            ));
        } else {
            trace.push(delivery_trace(&a.delivery, &run.status, None));
        }
        run.trace = trace;

        self.record_run(&run);
        self.bus.emit(
            EVENT_AUTOMATION_RUN,
            json!({
                "automationId": a.id, "name": a.name, "runId": run.id,
                "status": run.status, "reason": reason, "sessionId": run.session_id, "note": run.note,
            }),
        );
        if run.status != RUN_STATUS_STARTED {
            let outcome = run.status.clone();
            self.deliver(&a, &run, None, &outcome);
        }
        Ok(run)
    }

    /// Dry-run: replays the trigger with `payload` and returns the five-step trace.
    /// It starts, sends and records nothing; the guardrail reads the real 5h window.
    pub fn test_automation(&self, id: &str, payload: Value) -> anyhow::Result<Value> {
        let a = self
            .get(id)
            .ok_or_else(|| anyhow!("automation not found"))?;
        let now = now_iso();
        let event = payload.get("event").cloned();
        let mut rctx = json!({ "payload": payload });
        if let Some(ev) = &event {
            rctx["event"] = ev.clone();
        }
        rctx["automation"] = json!({ "id": a.id, "name": a.name });
        rctx["now"] = json!(now);

        let mut trace: Vec<TraceStep> = vec![trace_step(
            STEP_TRIGGER,
            OUTCOME_MATCHED,
            None,
            json!({ "reason": REASON_TEST }),
        )];

        let mut would_run = true;
        let mut status = DRY_RUN_WOULD_RUN.to_string();

        match &a.trigger {
            Trigger::Event { .. } => {
                let ty = event
                    .as_ref()
                    .and_then(|e| e["type"].as_str())
                    .unwrap_or("")
                    .to_string();
                let data = event
                    .as_ref()
                    .map(|e| e["data"].clone())
                    .unwrap_or(Value::Null);
                match event_trigger_matches(&a.trigger, &ty, &data) {
                    Ok(()) => {
                        trace.push(trace_step(STEP_FILTER, OUTCOME_PASSED, None, Value::Null))
                    }
                    Err(reason) => {
                        trace.push(trace_step(
                            STEP_FILTER,
                            OUTCOME_REJECTED,
                            Some(reason),
                            Value::Null,
                        ));
                        would_run = false;
                        status = DRY_RUN_FILTERED.to_string();
                    }
                }
            }
            Trigger::Webhook => trace.push(trace_step(
                STEP_FILTER,
                OUTCOME_NOT_APPLICABLE,
                Some("webhook triggers have no filter".to_string()),
                Value::Null,
            )),
            _ => trace.push(trace_step(
                STEP_FILTER,
                OUTCOME_NOT_APPLICABLE,
                Some("no filter concept for this trigger".to_string()),
                Value::Null,
            )),
        }

        match &a.action {
            Action::Launch {
                max_utilization, ..
            }
            | Action::Prompt {
                max_utilization, ..
            } => {
                if let Some(why) = self.utilization_block(*max_utilization) {
                    trace.push(trace_step(
                        STEP_GUARDRAIL,
                        OUTCOME_BLOCKED,
                        Some(why),
                        Value::Null,
                    ));
                    if would_run {
                        would_run = false;
                        status = OUTCOME_SKIPPED.to_string();
                    }
                } else {
                    trace.push(trace_step(
                        STEP_GUARDRAIL,
                        OUTCOME_PASSED,
                        None,
                        Value::Null,
                    ));
                }
            }
            _ => trace.push(trace_step(
                STEP_GUARDRAIL,
                OUTCOME_NOT_APPLICABLE,
                Some("not applicable: this action starts no session".to_string()),
                Value::Null,
            )),
        }

        trace.push(render_action_trace(&a.action, &rctx));

        let outcome_for_delivery = if would_run {
            RUN_STATUS_DONE
        } else {
            status.as_str()
        };
        trace.push(delivery_trace(
            &a.delivery,
            outcome_for_delivery,
            Some("hypothetical: dry-run, nothing sent"),
        ));

        Ok(json!({
            "automationId": a.id,
            "name": a.name,
            "dryRun": true,
            "wouldRun": would_run,
            "status": status,
            "trace": trace,
        }))
    }

    fn deliver(
        &self,
        a: &Automation,
        run: &AutomationRun,
        session: Option<&SessionRecord>,
        outcome: &str,
    ) {
        let Some(d) = &a.delivery else { return };
        if d.kind != DELIVERY_KIND_WEBHOOK {
            return;
        }
        let Some(url) = d.url.as_deref().filter(|u| !u.is_empty()) else {
            return;
        };
        if let Some(on) = &d.on {
            if !on.iter().any(|o| o == outcome) {
                return;
            }
        }
        let text = match session {
            Some(s) => {
                let mut t = format!(
                    "[aidp] {} — {} on {} → {}",
                    a.name,
                    s.template_id
                        .clone()
                        .unwrap_or_else(|| TEMPLATE_LABEL_PROMPT.to_string()),
                    s.project_name,
                    outcome
                );
                if let Some(u) = &s.usage {
                    t.push_str(&format!(
                        " ({}k tokens)",
                        (u.input + u.output + u.cache_read + u.cache_create) / 1000
                    ));
                }
                if outcome == OUTCOME_QUESTION {
                    if let Some(q) = &s.pending_question {
                        let qs: Vec<&str> = q
                            .questions
                            .as_array()
                            .map(|arr| arr.iter().filter_map(|x| x["question"].as_str()).collect())
                            .unwrap_or_default();
                        t.push('\n');
                        t.push_str(&qs.join(" | "));
                    }
                } else if let Some(lt) = &s.last_text {
                    t.push('\n');
                    t.push_str(&head(lt, 300));
                }
                t
            }
            None => format!(
                "[aidp] {} → {}{}",
                a.name,
                outcome,
                run.error
                    .as_ref()
                    .or(run.note.as_ref())
                    .map(|e| format!(": {e}"))
                    .unwrap_or_default()
            ),
        };
        let body = json!({
            "text": text, "outcome": outcome,
            "automation": { "id": a.id, "name": a.name },
            "run": run, "session": session,
        });
        if let Err(e) = crate::net::agent().post(url).send_json(body) {
            let detail = aidp_integrations::redact_url_in(&e.to_string(), url);
            self.bus.emit(
                EVENT_AUTOMATION_ERROR,
                json!({ "automationId": a.id, "name": a.name, "error": format!("delivery: {detail}") }),
            );
        }
    }

    fn on_bus_event(&self, ev: &Value) {
        let ty = ev["type"].as_str().unwrap_or("");
        let data = &ev["data"];

        if let Some(run_id) = data["automationRunId"].as_str() {
            if ty == EVENT_SESSION_ENDED || ty == EVENT_SESSION_QUESTION {
                let run = lock(&self.runs).get(run_id).cloned().or_else(|| {
                    self.list_runs(None, 10_000)
                        .into_iter()
                        .find(|r| r.id == run_id)
                });
                let a = data["automationId"].as_str().and_then(|id| self.get(id));
                let session = data["sessionId"]
                    .as_str()
                    .and_then(|id| self.sessions.get(id));
                if let (Some(mut run), Some(a)) = (run, a) {
                    if ty == EVENT_SESSION_ENDED {
                        run.status = match data["status"].as_str().unwrap_or("") {
                            RUN_STATUS_DONE => RUN_STATUS_DONE,
                            RUN_STATUS_CANCELLED => RUN_STATUS_CANCELLED,
                            _ => RUN_STATUS_FAILED,
                        }
                        .to_string();
                        run.ended_at = Some(now_iso());
                        if let Some(step) =
                            run.trace.iter_mut().rev().find(|s| s.step == STEP_DELIVERY)
                        {
                            *step = delivery_trace(&a.delivery, &run.status, None);
                        } else {
                            run.trace
                                .push(delivery_trace(&a.delivery, &run.status, None));
                        }
                        self.record_run(&run);
                        let outcome = run.status.clone();
                        self.deliver(&a, &run, session.as_ref(), &outcome);
                    } else {
                        self.deliver(&a, &run, session.as_ref(), OUTCOME_QUESTION);
                    }
                }
            }
            if ty == EVENT_SESSION_ENDED {
                self.release_pending(|pending_run_id, _| pending_run_id == run_id);
            }
        }

        if ty == SESSION_DEQUEUED_EVENT {
            self.release_pending_queue(data);
        }

        if ty.starts_with(AUTOMATION_EVENT_PREFIX) {
            return;
        }
        let candidates: Vec<Automation> = lock(&self.list)
            .iter()
            .filter(|a| a.enabled && matches!(a.trigger, Trigger::Event { .. }))
            .cloned()
            .collect();
        for a in candidates {
            if data["automationId"].as_str() == Some(a.id.as_str()) {
                continue;
            }
            if event_trigger_matches(&a.trigger, ty, data).is_err() {
                continue;
            }
            let ctx = json!({ "event": ev, "payload": data });
            if let Err(e) = self.run_automation(&a.id, REASON_EVENT, ctx) {
                self.bus.emit(
                    EVENT_AUTOMATION_ERROR,
                    json!({ "automationId": a.id, "name": a.name, "error": e.to_string() }),
                );
            }
        }
    }

    pub fn handle_webhook(
        &self,
        automation_id: &str,
        payload: Value,
    ) -> anyhow::Result<AutomationRun> {
        let a = self
            .get(automation_id)
            .filter(|a| matches!(a.trigger, Trigger::Webhook));
        let a = a.ok_or_else(|| anyhow!("no webhook automation with this id"))?;
        if !a.enabled {
            bail!("automation disabled");
        }
        self.bus.emit(
            EVENT_WEBHOOK_RECEIVED,
            json!({ "automationId": a.id, "name": a.name }),
        );
        self.run_automation(&a.id, REASON_WEBHOOK, json!({ "payload": payload }))
    }

    /// Handles a GitHub webhook delivery: emits `github.<event>` on the bus.
    pub fn handle_github(&self, event: &str, payload: &Value) -> Value {
        let s = |v: Option<&Value>| v.and_then(|x| x.as_str()).map(|x| x.to_string());
        let repo = s(payload.pointer("/repository/name")).unwrap_or_default();
        let full = s(payload.pointer("/repository/full_name")).unwrap_or_else(|| repo.clone());
        let action = s(payload.get("action")).unwrap_or_default();
        let pr = payload
            .pointer("/pull_request/number")
            .or_else(|| payload.pointer("/issue/number"))
            .or_else(|| payload.pointer("/check_run/pull_requests/0/number"))
            .or_else(|| payload.pointer("/workflow_run/pull_requests/0/number"))
            .and_then(|v| v.as_u64());
        let title = s(payload
            .pointer("/pull_request/title")
            .or_else(|| payload.pointer("/issue/title"))
            .or_else(|| payload.pointer("/workflow_run/name"))
            .or_else(|| payload.pointer("/check_run/name")));
        let conclusion = s(payload
            .pointer("/check_run/conclusion")
            .or_else(|| payload.pointer("/check_suite/conclusion"))
            .or_else(|| payload.pointer("/workflow_run/conclusion"))
            .or_else(|| payload.get("state")));
        let url = s(payload
            .pointer("/pull_request/html_url")
            .or_else(|| payload.pointer("/comment/html_url"))
            .or_else(|| payload.pointer("/check_run/html_url"))
            .or_else(|| payload.pointer("/workflow_run/html_url")));
        let project = self.projects.get(&repo);
        let relevant = match event {
            "pull_request" => matches!(
                action.as_str(),
                "opened" | "synchronize" | "reopened" | "ready_for_review" | "edited"
            ),
            "pull_request_review"
            | "pull_request_review_comment"
            | "pull_request_review_thread" => true,
            "issue_comment" => payload.pointer("/issue/pull_request").is_some(),
            "check_run" | "check_suite" | "workflow_run" => action == "completed",
            "status" => matches!(
                conclusion.as_deref(),
                Some("failure") | Some("error") | Some("success")
            ),
            _ => false,
        };
        let data = json!({
            "event": event, "action": action, "repo": full, "pr": pr, "title": title,
            "conclusion": conclusion, "sender": s(payload.pointer("/sender/login")), "url": url,
            "projectId": project.as_ref().map(|p| p.id.clone()),
            "projectName": project.as_ref().map(|p| p.name.clone()),
            "relevant": relevant,
        });
        self.bus
            .emit(&format!("{GITHUB_EVENT_PREFIX}{event}"), data.clone());
        data
    }
}

#[cfg(test)]
#[path = "../tests/unit/automations_tests.rs"]
mod automations_tests;
