//! The ticket-autopilot dispatcher: turns `todo`/`queued` tickets under `<vault>/TODO/` into sessions.
//! It only ever cancels its own queue entry and leaves the per-project limit to `start()`.

use crate::bus::Bus;
use crate::flows::{FlowEngine, FlowEngineError};
use crate::health::{self, SweptWorktree, WorktreeRemoveFailure, WorktreeSweepReport};
use crate::launcher::RunLauncher;
use crate::models::constants::flow_inputs::{
    INPUT_FEATURE, INPUT_ISSUE, INPUT_PROMPT, INPUT_TRACK,
};
use crate::models::constants::ticket_flow::FLOW_SESSION_PREFIX;
use crate::models::enums::native_run_status::NativeRunStatus;
use crate::models::enums::{FlowTicketReason, PrClassification, SessionStatus};
use crate::models::inbox_changed_event::InboxChangedEvent;
use crate::models::launched_run::LaunchedRun;
use crate::models::{
    ActionBody, AdoptResponse, DispatcherPausedChangedEvent, TicketStartedEvent, TicketStatusEvent,
};
use crate::native_runs::dispatch::autonomous_env;
use crate::projects::Projects;
use crate::prs::TICKET_BRANCH_PREFIXES;
use crate::session_error::SessionError;
use crate::sessions::{
    Launch, Origin, SessionManager, StartRequest, WorktreeRefused, EVENT_SESSION_ENDED,
};
use crate::sync::lock;
use crate::time::{now_iso, now_ms};
use crate::worktrees;
use aidp_core::clock::{Clock, SystemClock};
use aidp_core::process_runner::{OsProcessRunner, ProcessRunner};
use aidp_integrations::models::constants::branch::{FEATURE_BRANCH_PREFIX, FIX_BRANCH_PREFIX};
use aidp_integrations::pr::PrState as GhPrState;
use aidp_tickets::budget;
use aidp_tickets::dispatcher_state::{
    self, BudgetState, DispatcherState, LastPlan, RunningEntry, SkippedEntry, SlotsState,
};
use aidp_tickets::ledger::{self, Event};
use aidp_tickets::lifecycle;
use aidp_tickets::model::{self, Actor, IssueType, Status};
use aidp_tickets::models::enums::DispatcherLaunch;
use aidp_tickets::plan;
use aidp_tickets::store::{ListFilter, TicketStore};
use chrono::{DateTime, SecondsFormat, TimeZone, Utc};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, TryLockError};
use std::thread;
use std::time::Duration as StdDuration;
use tracing::{error, info, warn};

const TODO_DIR: &str = "TODO";
const STATE_DIR: &str = ".state";
const LAUNCH_DIR: &str = "launch";
const CONFIG_FILE: &str = "config.yaml";
const DISPATCHER_STATE_FILE: &str = "dispatcher.json";
const INPUTS_FILE_SUFFIX: &str = ".inputs.json";
const DEFAULT_TICK_SECONDS: u64 = 60;
const DISPATCHER_ORIGIN_KIND: &str = "dispatcher";
const RATE_LIMIT_REJECTED: &str = "rejected";
const TICK_REASON_TIMER: &str = "timer";
const TICK_REASON_SESSION_ENDED: &str = "session-ended";
const DISPATCH_REASON: &str = "dispatch";
const FORCED_OVER_BUDGET_REASON: &str = "forced-over-budget";
const FORCED_OVER_PAUSE_REASON: &str = "forced-over-pause";
const GATE_TIMEOUT_REASON: &str = "gate-timeout";
const QUEUE_RACE_REASON: &str = "queue-race";
const BUDGET_RESUME_REASON: &str = "budget";
const UNKNOWN_STAGE: &str = "?";
const EVENT_DISPATCHER_TICK: &str = "dispatcher.tick";
const EVENT_DISPATCHER_WAITING: &str = "dispatcher.waiting";
const EVENT_DISPATCHER_RESUMED: &str = "dispatcher.resumed";
const EVENT_DISPATCHER_PAUSED_CHANGED: &str = "dispatcher.paused_changed";
const EVENT_TICKET_STARTED: &str = "ticket.started";
const EVENT_TICKET_STATUS: &str = "ticket.status";
const EVENT_TICKET_REVIEWED: &str = "ticket.reviewed";
const EVENT_SESSION_QUESTION: &str = "session.question";
const EVENT_SESSION_ANSWERED: &str = "session.answered";
const EVENT_PR_MERGED: &str = "pr.merged";
const EVENT_WORKTREE_REMOVED: &str = "worktree.removed";
const EVENT_WORKTREE_REMOVE_FAILED: &str = "worktree.remove_failed";
const FIELD_SALVAGED_REF: &str = "salvagedRef";
const FIELD_PATH: &str = "path";
const FIELD_ATTEMPTS: &str = "attempts";
const FIELD_WARNINGS: &str = "warnings";
const HIDDEN_DIR_PREFIX: char = '.';
const GENERATED_DIR_PREFIX: char = '_';
const NO_PROJECT_FOR_REPO: &str = "no project registered for repo";
const WORKTREE_MISSING_ATTEMPTS: u32 = 0;
const WORKTREE_SALVAGE_ATTEMPTS: u32 = 1;
const PR_STAGE: &str = "pr";
const PR_DIR: &str = "pr";
const PR_FILE: &str = "pr.json";
const PR_URL_KEY: &str = "pr_url";
const URL_KEY: &str = "url";
const HTML_URL_KEY: &str = "html_url";
const NUMBER_KEY: &str = "number";
const STATE_KEY: &str = "state";
const QUESTIONS_KEY: &str = "questions";
const STAGE_TICK_MARKER: &str = "stage `";
const STAGE_TICK_END: char = '`';
const STAGE_COLON_MARKER: &str = "stage:";
const FIELD_TYPE: &str = "type";
const FIELD_DATA: &str = "data";
const WINDOW_FIVE_HOUR_KEY: &str = "5h";
const WINDOW_SEVEN_DAY_KEY: &str = "7d";
const WINDOW_FIVE_HOUR_LABEL_FR: &str = "5 h";
const WINDOW_SEVEN_DAY_LABEL_FR: &str = "7 j";
const VERDICT_ALLOW: &str = "allow";
const VERDICT_CONSERVATIVE: &str = "conservative";
const VERDICT_WAIT: &str = "wait";
const VERDICT_PAUSED: &str = "paused";
const FIELD_LAUNCHED: &str = "launched";
const FIELD_SKIPPED: &str = "skipped";
const FIELD_LAUNCHES: &str = "launches";
const FIELD_SLOTS_USED: &str = "slotsUsed";
const FIELD_SLOTS_MAX: &str = "slotsMax";
const FIELD_LAST_REASON: &str = "lastReason";
const FIELD_MODE: &str = "mode";
const FIELD_TICKETS_SYNCED: &str = "ticketsSynced";
const FIELD_ERROR: &str = "error";
const FIELD_UNTIL: &str = "until";
const FIELD_WINDOW: &str = "window";
const FIELD_UTILIZATION: &str = "utilization";
const FIELD_LIMIT: &str = "limit";
const FIELD_KEY: &str = "key";
const FIELD_STATUS: &str = "status";
const FIELD_SESSION_ID: &str = "sessionId";
const FIELD_PR: &str = "pr";
const FIELD_REASON: &str = "reason";
const FIELD_RESETS_AT: &str = "resetsAt";
const FIELD_IS_USING_OVERAGE: &str = "isUsingOverage";
const FIELD_FIVE_HOUR: &str = "fiveHour";
const FIELD_SEVEN_DAY: &str = "sevenDay";
const FIELD_OBSERVED_AT: &str = "observedAt";
const FIELD_VERDICT: &str = "verdict";
const FIELD_LIVE: &str = "live";
const FIELD_RESERVES: &str = "reserves";
const FIELD_LEARNED: &str = "learned";
const FIELD_UNREFLECTED: &str = "unreflected";
const FIELD_STATS: &str = "stats";

/// The `mode` the dispatcher reports and persists.
#[derive(Clone, Copy)]
enum DispatcherMode {
    Busy,
    NoVault,
    Paused,
    Waiting,
    Normal,
}

impl DispatcherMode {
    /// The wire slug of the mode.
    const fn as_str(self) -> &'static str {
        match self {
            DispatcherMode::Busy => "busy",
            DispatcherMode::NoVault => "no-vault",
            DispatcherMode::Paused => "paused",
            DispatcherMode::Waiting => "waiting",
            DispatcherMode::Normal => "normal",
        }
    }
}

/// The wire key of a rate-limit window.
const fn window_key(window: &budget::WindowKind) -> &'static str {
    match window {
        budget::WindowKind::FiveHour => WINDOW_FIVE_HOUR_KEY,
        budget::WindowKind::SevenDay => WINDOW_SEVEN_DAY_KEY,
    }
}

/// The French label of a rate-limit window, as `budget` words its verdicts.
const fn window_label_fr(window: &budget::WindowKind) -> &'static str {
    match window {
        budget::WindowKind::FiveHour => WINDOW_FIVE_HOUR_LABEL_FR,
        budget::WindowKind::SevenDay => WINDOW_SEVEN_DAY_LABEL_FR,
    }
}

/// Emits `ticket.status` on the bus from the typed payload.
fn emit_ticket_status(bus: &Bus, key: &str, status: &str) {
    match serde_json::to_value(TicketStatusEvent { key, status }) {
        Ok(payload) => {
            bus.emit(EVENT_TICKET_STATUS, payload);
        }
        Err(e) => error!("cannot serialize ticket.status for {key}: {e}"),
    }
}

/// The feature branch a ticket's session works on.
fn feature_branch(key: &str) -> String {
    worktrees::branch_for(key)
}

/// The branch a ticket's flow works on: `fix/` for a bug, `feat/` otherwise.
fn branch_for_ticket(issue_type: IssueType, key: &str) -> String {
    let prefix = if issue_type == IssueType::Bug {
        FIX_BRANCH_PREFIX
    } else {
        FEATURE_BRANCH_PREFIX
    };
    format!("{prefix}{key}")
}

/// `<vault>/TODO/.state/dispatcher.json`.
fn dispatcher_state_path(vault_root: &Path) -> PathBuf {
    vault_root
        .join(TODO_DIR)
        .join(STATE_DIR)
        .join(DISPATCHER_STATE_FILE)
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

/// Gap between two merge probes of the in-review tickets.
const MERGE_PROBE_INTERVAL_MS: i64 = 10 * 60_000;

/// How long `Dispatcher::catalog()` may serve a cached `TemplateCatalog`.
/// Templates change far less often than tickets, so five minutes is accepted.
const TEMPLATE_CATALOG_REFRESH_MS: i64 = 300_000;

/// The `mode` a tick reports when another tick already holds the tick lock:
/// it returns at once, touching neither the store nor the persisted state.
const BUSY_MODE: &str = DispatcherMode::Busy.as_str();

/// The report of a tick skipped because another one holds the tick lock.
fn busy_report(reason: &str) -> Value {
    json!({ FIELD_MODE: BUSY_MODE, FIELD_LAST_REASON: reason, FIELD_LAUNCHED: [], FIELD_SKIPPED: [] })
}

/// Wire slug for a launch skipped because the dispatcher's own slot ceiling is full.
/// It has no [`aidp_tickets::SkipReason`] variant of its own.
const NO_SLOT_REASON: &str = "no-slot";

/// How the dispatcher reaches git and waits between delete attempts.
/// Tests inject a scripted runner and a sleeper that returns at once.
pub(crate) struct WorktreeOps {
    runner: Arc<dyn ProcessRunner>,
    sleeper: Arc<dyn Fn(StdDuration) + Send + Sync>,
}

impl WorktreeOps {
    /// Real git processes and a real sleep.
    pub(crate) fn system() -> Self {
        WorktreeOps {
            runner: Arc::new(OsProcessRunner),
            sleeper: Arc::new(thread::sleep),
        }
    }
}

/// The cap warning of a manual start over the per-project worktree cap, `None` when it fits.
fn manual_cap_warning(holders: &[String], max: usize, key: &str) -> Option<String> {
    let has_own = holders.iter().any(|holder| holder == key);
    match plan::cap_verdict(holders, max, has_own) {
        plan::CapVerdict::Allow => None,
        plan::CapVerdict::Cap(keys) => Some(dispatcher_state::worktree_cap_warning(max, &keys)),
    }
}

/// Manual starts bypass the worktree cap; the returned map holds the warning per ticket key.
fn exempt_manual_starts(
    candidates: &[aidp_tickets::model::Ticket],
    holders: &mut HashMap<String, Vec<String>>,
    max: usize,
) -> HashMap<String, String> {
    let mut warnings = HashMap::new();
    for ticket in candidates
        .iter()
        .filter(|ticket| ticket.autopilot.manual_priority == Some(true))
    {
        let service_holders = holders.entry(ticket.service.clone()).or_default();
        if let Some(warning) = manual_cap_warning(service_holders, max, &ticket.key) {
            warnings.insert(ticket.key.clone(), warning);
            service_holders.push(ticket.key.clone());
        }
    }
    warnings
}

/// A session the dispatcher started during one tick.
struct LaunchedSession {
    key: String,
    session_id: String,
    service: String,
}

/// `report` with a `warnings` array, left untouched when there is no warning.
fn with_warnings(mut report: Value, warnings: &[String]) -> Value {
    if !warnings.is_empty() {
        report[FIELD_WARNINGS] = json!(warnings);
    }
    report
}

/// What deleting a ticket's worktree did.
enum WorktreeRemoval {
    Absent,
    Removed(Option<String>),
    Failed(WorktreeRemoveFailure),
}

/// The health entry describing why `path` could not be deleted.
fn remove_failure(path: &Path, key: &str, error: &worktrees::RemoveError) -> WorktreeRemoveFailure {
    let attempts = match error {
        worktrees::RemoveError::Locked { attempts, .. } => *attempts,
        worktrees::RemoveError::Git(_) | worktrees::RemoveError::InvalidKey(_) => {
            WORKTREE_SALVAGE_ATTEMPTS
        }
    };
    WorktreeRemoveFailure {
        key: key.to_string(),
        path: path.to_string_lossy().to_string(),
        error: error.to_string(),
        attempts,
    }
}

/// Visible child directories of `dir`, as `(name, path)`; dot and underscore names are skipped.
fn visible_subdirs(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|entry| entry.path().is_dir())
        .filter_map(|entry| {
            let name = entry.file_name().to_string_lossy().to_string();
            (!name.starts_with(HIDDEN_DIR_PREFIX) && !name.starts_with(GENERATED_DIR_PREFIX))
                .then(|| (name, entry.path()))
        })
        .collect()
}

/// Every `<vault>/Pipelines/<repo>/<KEY>/.worktree` on disk, as `(repo, key, path)`.
fn worktree_dirs(vault_root: &Path) -> Vec<(String, String, PathBuf)> {
    let mut found = Vec::new();
    for (repo, repo_dir) in visible_subdirs(&vault_root.join(worktrees::PIPELINES_DIR)) {
        for (key, key_dir) in visible_subdirs(&repo_dir) {
            let path = key_dir.join(worktrees::WORKTREE_DIR);
            if path.is_dir() {
                found.push((repo.clone(), key, path));
            }
        }
    }
    found
}

/// A forced start: its outcome, the session it started and its worktree-cap warning.
struct ForcedLaunch {
    outcome: LaunchAttemptOutcome,
    session: Option<LaunchedSession>,
    warning: Option<String>,
}

impl ForcedLaunch {
    /// A forced start that ended before any session was started.
    fn without_session(outcome: LaunchAttemptOutcome) -> Self {
        ForcedLaunch {
            outcome,
            session: None,
            warning: None,
        }
    }
}

/// What [`Dispatcher::build_plan`] computed for one tick.
struct BuiltPlan {
    dispatch_plan: plan::DispatchPlan,
    running: Vec<RunningEntry>,
    updated_at: HashMap<String, DateTime<Utc>>,
    rich_verdict: budget::Verdict,
    budget_state: BudgetState,
    manual_warnings: HashMap<String, String>,
}

/// The `StartRequest` of one dispatcher launch: always inside the ticket's own worktree.
fn dispatcher_start_request(
    project_id: &str,
    launch: &plan::Launch,
    summary: &str,
    inputs_path: &Path,
    cfg: &aidp_tickets::config::DispatcherConfig,
    epic_block: Option<String>,
    engine: Option<&str>,
) -> StartRequest {
    StartRequest {
        prompt: epic_block,
        project_id: project_id.to_string(),
        engine: engine.map(str::to_string),
        template_id: Some(launch.template.clone()),
        target: Some(launch.key.clone()),
        inputs_file: Some(inputs_path.to_string_lossy().to_string()),
        permission_mode: Some(cfg.permission_mode.clone()),
        model: Some(cfg.model.clone()),
        auto_close: Some(true),
        origin: Some(Origin {
            kind: DISPATCHER_ORIGIN_KIND.to_string(),
            r#ref: Some(launch.key.clone()),
            label: Some(summary.to_string()),
        }),
        auto_approve_gates: Some(cfg.auto_approve_gates.clone()),
        worktree: Some(launch.key.clone()),
        env: autonomous_env(),
        ..Default::default()
    }
}

/// The outcome of a `start()` that failed: a branch held elsewhere is its own skip reason.
fn start_failure_outcome(error: &anyhow::Error) -> LaunchAttemptOutcome {
    match error.downcast_ref::<WorktreeRefused>() {
        Some(refused) if refused.slug == plan::SKIP_BRANCH_CHECKED_OUT_ELSEWHERE => {
            skipped_with_detail(
                plan::SKIP_BRANCH_CHECKED_OUT_ELSEWHERE,
                refused.detail.clone(),
            )
        }
        _ => LaunchAttemptOutcome::Skipped {
            reason: aidp_tickets::SkipReason::StartFailed.into(),
            detail: Some(error.to_string()),
        },
    }
}

/// True while a native run has not reached a final status.
fn is_live_native(status: NativeRunStatus) -> bool {
    native_outcome(status).is_none()
}

/// The ticket outcome of a final native run status, `None` while it is still live.
fn native_outcome(status: NativeRunStatus) -> Option<lifecycle::SessionOutcome> {
    match status {
        NativeRunStatus::Done => Some(lifecycle::SessionOutcome::Done),
        NativeRunStatus::Failed => Some(lifecycle::SessionOutcome::Failed {
            rate_limited: false,
        }),
        NativeRunStatus::Cancelled => Some(lifecycle::SessionOutcome::Cancelled),
        NativeRunStatus::Pending
        | NativeRunStatus::Running
        | NativeRunStatus::WaitingGate
        | NativeRunStatus::Paused => None,
    }
}

/// The references to try, in order, to find the PR of ticket `key`.
/// A flow-owned ticket tries its recorded PR number first, then `feat/<key>`, then `fix/<key>`.
fn merge_probe_references(key: &str, recorded: Option<u64>, flow_owned: bool) -> Vec<String> {
    let recorded_first = recorded
        .filter(|_| flow_owned)
        .map(|number| number.to_string());
    recorded_first
        .into_iter()
        .chain(
            TICKET_BRANCH_PREFIXES
                .iter()
                .map(|prefix| format!("{prefix}{key}")),
        )
        .collect()
}

/// The first PR view `lookup` finds for `references`.
fn first_pr_view(references: &[String], lookup: impl Fn(&str) -> Option<Value>) -> Option<Value> {
    references.iter().find_map(|reference| lookup(reference))
}

/// The final PR state a `gh pr view` JSON reports, `None` while it is still open.
fn final_pr_state(view: &Value) -> Option<lifecycle::PrState> {
    let state = view
        .get(STATE_KEY)
        .and_then(|state| serde_json::from_value::<GhPrState>(state.clone()).ok())?;
    match state {
        GhPrState::Merged => Some(lifecycle::PrState::Merged),
        GhPrState::Closed => Some(lifecycle::PrState::Closed),
        _ => None,
    }
}

/// The resident dispatcher.
pub struct Dispatcher {
    /// Vault root from `aidp_core::vault::resolve_global`; empty when no vault is configured.
    /// Every tick then no-ops after a one-time warning.
    vault_root: PathBuf,
    sessions: Arc<SessionManager>,
    /// Routes launches to native runs; `None` keeps every launch a legacy session.
    launcher: Option<Arc<RunLauncher>>,
    /// Engine that runs the delivery flow when `dispatcher.launch` is `flow`.
    flows: Mutex<Option<Arc<FlowEngine>>>,
    pub(crate) projects: Arc<Projects>,
    pub(crate) bus: Arc<Bus>,
    /// The last tick's (or the last restart's) `.state/dispatcher.json`,
    /// mirrored in memory so `state_json()` never re-reads disk.
    state: Mutex<DispatcherState>,
    skill_root: Option<PathBuf>,
    /// Epoch millis of the last merge probe pass, `None` before
    /// the first one this process has run.
    last_sync: Mutex<Option<i64>>,
    /// `(built_at_ms, catalog)`, refreshed at most every
    /// [`TEMPLATE_CATALOG_REFRESH_MS`] — see [`Dispatcher::catalog`].
    catalog_cache: Mutex<Option<(i64, plan::TemplateCatalog)>>,
    /// Serializes `tick_inner`: the timer thread, the bus subscriber and any
    /// HTTP thread may all tick, but only one may run the body at a time.
    tick_lock: Mutex<()>,
    /// Fires the "no vault configured" warning exactly once.
    warned_no_vault: AtomicBool,
    /// Consumed by the first non-paused tick: gates the restart/orphan sweep
    /// so it runs exactly once per process lifetime.
    first_tick: AtomicBool,
    /// Git runner and sleeper behind every worktree creation check and deletion.
    worktree_ops: WorktreeOps,
}

/// What [`Dispatcher::resolve_bus_event`] resolves a bus event's `sessionId` into.
/// It carries the open store, the ticket with its body, its key and the session record.
struct ResolvedBusEvent {
    store: TicketStore,
    ticket: aidp_tickets::model::Ticket,
    body: String,
    extra: serde_yaml::Mapping,
    key: String,
    rec: crate::sessions::SessionRecord,
}

/// The ticket key of a session started by the dispatcher, `None` for any other origin.
/// A session without the dispatcher origin kind or a ticket key is never adopted.
fn dispatcher_ticket_key(origin: &Origin) -> Option<&str> {
    if origin.kind != DISPATCHER_ORIGIN_KIND {
        return None;
    }
    origin.r#ref.as_deref()
}

/// Journal-ready event for a dispatcher status transition, or `None` when the guard refuses it.
/// On refusal it logs to stderr and the caller must leave the ticket untouched: no save, no ledger entry.
fn guard_and_build_event(
    context: &str,
    key: &str,
    from: Status,
    to: Status,
    reason: String,
    session_id: Option<String>,
) -> Option<Event> {
    match model::guard(from, to, Actor::Dispatcher, false) {
        Ok(()) => Some(Event {
            ts: SystemClock.now_utc(),
            key: key.to_string(),
            from: Some(from),
            to,
            actor: Actor::Dispatcher,
            reason: Some(reason),
            session_id,
        }),
        Err(e) => {
            warn!("{context} {key}: {e}");
            None
        }
    }
}

/// Escape every non-ASCII character of JSON `text` as `\uXXXX`, surrogate pairs included.
/// ASCII bytes, including the JSON structure, are left untouched.
fn ascii_escape_json(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        if ch.is_ascii() {
            out.push(ch);
        } else {
            let mut buf = [0u16; 2];
            for unit in ch.encode_utf16(&mut buf) {
                out.push_str(&format!("\\u{unit:04x}"));
            }
        }
    }
    out
}

/// Write `value` as ASCII-escaped pretty JSON to `path` atomically (tmp + rename).
/// Accented or emoji ticket bodies are otherwise mangled on the Windows command line.
fn write_inputs_file(path: &Path, value: &Value) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = serde_json::to_string_pretty(value)?;
    let escaped = ascii_escape_json(&text);
    let tmp = path.with_extension(format!("json.tmp{}", std::process::id()));
    std::fs::write(&tmp, &escaped)?;
    if path.exists() {
        let _ = std::fs::remove_file(path);
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

const PAUSE_SECTION_KEY: &str = "dispatcher";
const PAUSE_LEAF_KEY: &str = "paused";

/// The existing `config.yaml` text with only `dispatcher.paused` changed, comments kept.
/// `None` when the file is missing, out of the patch's reach, or does not read back as asked.
fn patched_paused_text(path: &Path, paused: bool) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let edit = crate::settings_yaml::LeafEdit {
        path: vec![PAUSE_SECTION_KEY.to_string(), PAUSE_LEAF_KEY.to_string()],
        value: Some(serde_yaml::Value::Bool(paused)),
    };
    let patched = crate::settings_yaml::patch_leaves(&text, &[edit])?;
    let read_back: serde_yaml::Value = serde_yaml::from_str(&patched).ok()?;
    (read_back
        .get(PAUSE_SECTION_KEY)?
        .get(PAUSE_LEAF_KEY)?
        .as_bool()?
        == paused)
        .then_some(patched)
}

/// tmp + rename write of plain text — the same atomic-write idiom as
/// `write_inputs_file`, for `TODO/config.yaml` (`Dispatcher::set_paused`).
fn write_text_atomic(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension(format!("yaml.tmp{}", std::process::id()));
    std::fs::write(&tmp, text)?;
    if path.exists() {
        let _ = std::fs::remove_file(path);
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Read `pr.json`, the first hit in the PR search order.
/// No schema governs it, so `url`, `html_url` and `pr_url` are all accepted.
fn read_pr_json(path: &Path) -> Option<lifecycle::PrInfo> {
    let text = std::fs::read_to_string(path).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    let url = v
        .get(URL_KEY)
        .or_else(|| v.get(HTML_URL_KEY))
        .or_else(|| v.get(PR_URL_KEY))
        .and_then(Value::as_str)?
        .to_string();
    Some(lifecycle::PrInfo {
        url,
        number: v.get(NUMBER_KEY).and_then(Value::as_u64),
        state: v.get(STATE_KEY).and_then(Value::as_str).map(str::to_string),
    })
}

/// `outputs.pr_url` on the `pr` stage of `run-state.json`.
/// A prose convention only, so a miss is normal and never logged.
fn read_pr_url_from_run_state(run_dir: &Path) -> Option<lifecycle::PrInfo> {
    let state = crate::runs::read_run_state(run_dir)?;
    let pr_stage = state.stages.iter().find(|stage| stage.name == PR_STAGE)?;
    let url = pr_stage
        .outputs
        .get(PR_URL_KEY)?
        .to_string_lossy()
        .into_owned();
    Some(lifecycle::PrInfo {
        url,
        number: None,
        state: None,
    })
}

/// `changes::pr_for_branch`'s projected JSON (`number,title,state,url,...`)
/// into the domain's `PrInfo`.
fn json_value_to_pr_info(v: &Value) -> lifecycle::PrInfo {
    lifecycle::PrInfo {
        url: v
            .get(URL_KEY)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        number: v.get(NUMBER_KEY).and_then(Value::as_u64),
        state: v.get(STATE_KEY).and_then(Value::as_str).map(str::to_string),
    }
}

/// Fallback when no run has a pending gate: find a stage mention in the question text.
/// Returns `None` when nothing matches.
fn parse_stage_from_question_text(data: &Value) -> Option<String> {
    let questions = data.get(QUESTIONS_KEY)?.as_array()?;
    for q in questions {
        let Some(text) = q.as_str() else { continue };
        if let Some(idx) = text.find(STAGE_TICK_MARKER) {
            let rest = &text[idx + STAGE_TICK_MARKER.len()..];
            if let Some(end) = rest.find(STAGE_TICK_END) {
                return Some(rest[..end].to_string());
            }
        }
        if let Some(idx) = text.find(STAGE_COLON_MARKER) {
            let rest = text[idx + STAGE_COLON_MARKER.len()..].trim_start();
            let token: String = rest.chars().take_while(|c| !c.is_whitespace()).collect();
            if !token.is_empty() {
                return Some(token);
            }
        }
    }
    None
}

/// The wire slug a skipped launch reports: a dispatcher, plan or slot-ceiling reason.
/// It is never re-derived; it is whatever the originating type already spells.
enum SkipReasonWire {
    Dispatcher(aidp_tickets::SkipReason),
    Plan(plan::SkipReason),
    Other(String),
}

impl fmt::Display for SkipReasonWire {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SkipReasonWire::Dispatcher(reason) => write!(f, "{reason}"),
            SkipReasonWire::Plan(reason) => write!(f, "{}", reason.as_str()),
            SkipReasonWire::Other(reason) => write!(f, "{reason}"),
        }
    }
}

impl From<aidp_tickets::SkipReason> for SkipReasonWire {
    fn from(reason: aidp_tickets::SkipReason) -> Self {
        SkipReasonWire::Dispatcher(reason)
    }
}

impl From<&plan::SkipReason> for SkipReasonWire {
    fn from(reason: &plan::SkipReason) -> Self {
        SkipReasonWire::Plan(reason.clone())
    }
}

impl From<String> for SkipReasonWire {
    fn from(reason: String) -> Self {
        SkipReasonWire::Other(reason)
    }
}

impl From<&str> for SkipReasonWire {
    fn from(reason: &str) -> Self {
        SkipReasonWire::Other(reason.to_string())
    }
}

/// The outcome of one [`aidp_tickets::plan::Launch`] execution attempt.
enum LaunchAttemptOutcome {
    Launched(String),
    Skipped {
        reason: SkipReasonWire,
        detail: Option<String>,
    },
}

/// A `Skipped` outcome carrying no extra detail beyond its reason.
fn skipped(reason: impl Into<SkipReasonWire>) -> LaunchAttemptOutcome {
    LaunchAttemptOutcome::Skipped {
        reason: reason.into(),
        detail: None,
    }
}

/// A `Skipped` outcome pairing a stable reason slug with a human-readable `detail`.
/// Callers match on `reason` without parsing prose.
fn skipped_with_detail(
    reason: impl Into<SkipReasonWire>,
    detail: impl Into<String>,
) -> LaunchAttemptOutcome {
    LaunchAttemptOutcome::Skipped {
        reason: reason.into(),
        detail: Some(detail.into()),
    }
}

/// The outcome of submitting a launch to `SessionManager::start`.
pub(crate) enum LaunchOutcome {
    Started(String),
    Requeued(String),
}

/// Reconcile a `SessionManager::start` outcome, cancelling the dispatcher's own entry on `Queued`.
/// The cancel is a closure so tests need no real `claude` process; the ticket is replayed next tick.
pub(crate) fn reconcile_launch<C: FnOnce(&str)>(launch: &Launch, cancel: C) -> LaunchOutcome {
    match launch {
        Launch::Started(rec) => LaunchOutcome::Started(rec.id.clone()),
        Launch::Queued(q) => {
            cancel(&q.id);
            LaunchOutcome::Requeued(q.id.clone())
        }
    }
}

/// `ms` (Unix milliseconds) to RFC3339, or `None` outside chrono's representable range.
fn millis_to_rfc3339(ms: i64) -> Option<String> {
    Utc.timestamp_millis_opt(ms)
        .single()
        .map(|dt| dt.to_rfc3339_opts(SecondsFormat::Millis, true))
}

/// The inverse of [`millis_to_rfc3339`]: an RFC3339 string back to Unix
/// milliseconds, or `None` if it does not parse.
fn parse_rfc3339_ms(s: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.timestamp_millis())
}

/// The live `RateLimitInfo` as a `budget::RateSnapshot`, renaming fields with no unit conversion.
/// `resets_at` stays in seconds and `observed_at` in milliseconds.
fn to_rate_snapshot(info: &crate::sessions::RateLimitInfo) -> budget::RateSnapshot {
    let window = |w: &crate::sessions::RateLimitWindow| budget::Window {
        utilization: w.utilization,
        resets_at_s: w.resets_at,
    };
    budget::RateSnapshot {
        status: info.status.clone(),
        is_using_overage: info.is_using_overage,
        five_hour: info.five_hour.as_ref().map(window),
        seven_day: info.seven_day.as_ref().map(window),
        observed_at_ms: info.observed_at,
    }
}

/// Adapt the rich `budget::Verdict` into the flat `plan::Verdict` that `plan::plan` consumes.
/// `plan::plan` already prefixes budget skips with `budget:`, so the detail stays short.
fn to_plan_verdict(v: &budget::Verdict) -> plan::Verdict {
    match v {
        budget::Verdict::Allow { max_launches, .. } => plan::Verdict {
            max_launches: *max_launches,
            reason: Some(format!("budget allows {max_launches} more launch(es)")),
        },
        budget::Verdict::Conservative { max_launches, why } => plan::Verdict {
            max_launches: *max_launches,
            reason: Some((*why).to_string()),
        },
        budget::Verdict::Wait {
            window,
            utilization,
            limit,
            ..
        } => {
            let window_label = window_key(window);
            plan::Verdict {
                max_launches: 0,
                reason: Some(format!(
                    "{window_label} window at {}% >= {}%",
                    (*utilization * 100.0).round(),
                    (*limit * 100.0).round()
                )),
            }
        }
        budget::Verdict::Paused { why } => plan::Verdict {
            max_launches: 0,
            reason: Some(why.clone()),
        },
    }
}

/// Build the persisted `BudgetState` from the tick's verdict, snapshot and reserves.
/// Utilizations are the raw snapshot values, never the projected ones of `Wait`.
fn to_budget_state(
    v: &budget::Verdict,
    snap: Option<&budget::RateSnapshot>,
    reserve_5h: f64,
    reserve_7d: f64,
    cfg: &aidp_tickets::config::BudgetConfig,
    unreflected: usize,
) -> BudgetState {
    let utilization_5h = snap
        .and_then(|s| s.five_hour.as_ref())
        .map(|w| w.utilization)
        .unwrap_or(0.0);
    let utilization_7d = snap
        .and_then(|s| s.seven_day.as_ref())
        .map(|w| w.utilization)
        .unwrap_or(0.0);
    let resets_at_5h = snap
        .and_then(|s| s.five_hour.as_ref())
        .and_then(|w| millis_to_rfc3339(w.resets_at_s * 1000));
    let observed_at = snap.and_then(|s| millis_to_rfc3339(s.observed_at_ms));

    let (verdict, max_launches, why, wait_window) = match v {
        budget::Verdict::Allow { max_launches, .. } => (
            VERDICT_ALLOW.to_string(),
            *max_launches,
            String::new(),
            None,
        ),
        budget::Verdict::Conservative { max_launches, why } => (
            VERDICT_CONSERVATIVE.to_string(),
            *max_launches,
            (*why).to_string(),
            None,
        ),
        budget::Verdict::Wait {
            window,
            utilization,
            limit,
            ..
        } => {
            let (label, key) = (window_label_fr(window), window_key(window));
            (
                VERDICT_WAIT.to_string(),
                0,
                format!(
                    "fenêtre {label} à {}% ≥ {}%",
                    (*utilization * 100.0).round(),
                    (*limit * 100.0).round()
                ),
                Some(key.to_string()),
            )
        }
        budget::Verdict::Paused { why } => (VERDICT_PAUSED.to_string(), 0, why.clone(), None),
    };

    BudgetState {
        verdict,
        max_launches,
        why,
        utilization_5h,
        utilization_7d,
        limit_5h: cfg.max_utilization_5h,
        limit_7d: cfg.max_utilization_7d,
        reserve_5h,
        reserve_7d,
        unreflected,
        resets_at_5h,
        wait_window,
        observed_at,
    }
}

impl Dispatcher {
    /// An empty `vault_root` means no vault is configured.
    /// Otherwise the last persisted `dispatcher.json` seeds `state_json()` after a restart.
    pub fn new(
        vault_root: PathBuf,
        sessions: Arc<SessionManager>,
        projects: Arc<Projects>,
        bus: Arc<Bus>,
    ) -> Arc<Self> {
        Self::build(vault_root, sessions, None, projects, bus)
    }

    /// Same as [`Dispatcher::new`], launching through `launcher` so a ticket starts a native run.
    pub fn new_with_launcher(
        vault_root: PathBuf,
        sessions: Arc<SessionManager>,
        launcher: Arc<RunLauncher>,
        projects: Arc<Projects>,
        bus: Arc<Bus>,
    ) -> Arc<Self> {
        Self::build(vault_root, sessions, Some(launcher), projects, bus)
    }

    /// Hands the dispatcher the engine it starts delivery flows on.
    pub fn attach_flows(&self, engine: Arc<FlowEngine>) {
        *lock(&self.flows) = Some(engine);
    }

    fn build(
        vault_root: PathBuf,
        sessions: Arc<SessionManager>,
        launcher: Option<Arc<RunLauncher>>,
        projects: Arc<Projects>,
        bus: Arc<Bus>,
    ) -> Arc<Self> {
        let initial_state = if vault_root.as_os_str().is_empty() {
            DispatcherState::default()
        } else {
            let path = dispatcher_state_path(&vault_root);
            crate::store::read_json(&path, DispatcherState::default())
        };
        let skill_root = sessions.configured_skill_root();
        Arc::new(Dispatcher {
            vault_root,
            sessions,
            launcher,
            flows: Mutex::new(None),
            projects,
            bus,
            state: Mutex::new(initial_state),
            skill_root,
            last_sync: Mutex::new(None),
            catalog_cache: Mutex::new(None),
            tick_lock: Mutex::new(()),
            warned_no_vault: AtomicBool::new(false),
            first_tick: AtomicBool::new(true),
            worktree_ops: WorktreeOps::system(),
        })
    }

    /// Spawn the tick timer thread and the bus subscriber thread reacting to session events.
    /// The tick interval is re-read every cycle so a config edit needs no restart.
    pub fn spawn(self: &Arc<Self>, shutdown: Arc<crate::shutdown::Shutdown>) {
        {
            let me = self.clone();
            let shutdown_tick = shutdown.clone();
            thread::spawn(move || {
                let _loop_guard = crate::threads::enter_loop(crate::threads::DISPATCHER_TIMER);
                crate::threads::guarded(crate::threads::DISPATCHER_TIMER, || me.sweep_worktrees());
                loop {
                    if shutdown_tick.is_set() {
                        break;
                    }
                    let secs = me.sleep_secs();
                    if !sleep_unless_stopped(&shutdown_tick, StdDuration::from_secs(secs)) {
                        break;
                    }
                    crate::threads::guarded(crate::threads::DISPATCHER_TIMER, || {
                        me.tick(TICK_REASON_TIMER)
                    });
                }
            });
        }
        {
            let me = self.clone();
            let shutdown_bus = shutdown.clone();
            let rx = self.bus.subscribe();
            thread::spawn(move || {
                let _loop_guard = crate::threads::enter_loop(crate::threads::DISPATCHER_BUS);
                for ev in rx.iter() {
                    if shutdown_bus.is_set() {
                        break;
                    }
                    crate::threads::guarded(crate::threads::DISPATCHER_BUS, || {
                        me.on_bus_event(&ev)
                    });
                }
            });
        }
    }

    /// Seconds the timer thread sleeps: `tick_seconds`, or the time left until `wait_until`, at least 1.
    /// A wait decided at tick N is only re-evaluated once the current sleep elapses.
    fn sleep_secs(&self) -> u64 {
        let base = self.tick_interval_secs();
        let (mode, wait_until) = {
            let s = lock(&self.state);
            (s.mode.clone(), s.wait_until.clone())
        };
        if mode != DispatcherMode::Waiting.as_str() {
            return base;
        }
        let Some(until_ms) = wait_until.as_deref().and_then(parse_rfc3339_ms) else {
            return base;
        };
        let remaining_secs = ((until_ms - now_ms()).max(0) / 1000) as u64;
        base.min(remaining_secs).max(1)
    }

    fn tick_interval_secs(&self) -> u64 {
        if self.vault_root.as_os_str().is_empty() {
            return DEFAULT_TICK_SECONDS;
        }
        match TicketStore::open(&self.vault_root) {
            Ok(store) => u64::from(store.config().dispatcher.tick_seconds.max(1)),
            Err(_) => DEFAULT_TICK_SECONDS,
        }
    }

    /// Rebuild the `TemplateCatalog` at most once per refresh window, else serve the cache.
    /// A template edit takes up to five minutes to show; `aidp ticket plan` always reads fresh.
    fn catalog(&self) -> plan::TemplateCatalog {
        let now = now_ms();
        {
            let cache = lock(&self.catalog_cache);
            if let Some((built_at, cached)) = cache.as_ref() {
                if now - built_at < TEMPLATE_CATALOG_REFRESH_MS {
                    return cached.clone();
                }
            }
        }
        let fresh = match &self.skill_root {
            Some(root) => plan::TemplateCatalog::from_skill_root(root),
            None => plan::TemplateCatalog::empty(),
        };
        *lock(&self.catalog_cache) = Some((now, fresh.clone()));
        fresh
    }

    /// Build the `DispatchPlan` a real tick would, with running entries and plan-time `updated` snapshots.
    /// Shared by `tick` and `plan_only` so they never disagree about what would run.
    fn build_plan(&self, store: &TicketStore) -> BuiltPlan {
        let running_records = self.sessions.list(Some(SessionStatus::Running), None, 0);

        let running_slots: Vec<plan::RunningSlot> = running_records
            .iter()
            .map(|rec| plan::RunningSlot {
                project: rec.project_name.clone(),
                origin_kind: rec.origin.kind.clone(),
                ticket_key: dispatcher_ticket_key(&rec.origin).map(str::to_string),
                started_at_ms: DateTime::parse_from_rfc3339(&rec.started_at)
                    .map(|d| d.timestamp_millis())
                    .unwrap_or(0),
            })
            .collect();

        let state_running = Self::running_entries_from(&running_records);

        let now = now_ms();
        let snap = self.sessions.rate_limit().as_ref().map(to_rate_snapshot);

        let running_dispatcher = running_slots
            .iter()
            .filter(|s| s.origin_kind == DISPATCHER_ORIGIN_KIND)
            .count();
        let unreflected = match snap.as_ref() {
            Some(s) => running_slots
                .iter()
                .filter(|slot| {
                    slot.origin_kind == DISPATCHER_ORIGIN_KIND
                        && slot.started_at_ms > s.observed_at_ms
                })
                .count(),
            None => 0,
        };

        let mut candidates = store.list(&ListFilter {
            status: vec![Status::Todo, Status::Queued],
            ..ListFilter::default()
        });
        // The test lane also takes the issues whose dev flow ended: in progress, phase `to-test`.
        candidates.extend(
            store
                .list(&ListFilter {
                    status: vec![Status::InProgress],
                    ..ListFilter::default()
                })
                .into_iter()
                .filter(|t| t.autopilot.phase() == model::Phase::ToTest),
        );
        let plan_updated_at: HashMap<String, DateTime<Utc>> = candidates
            .iter()
            .map(|t| (t.key.clone(), t.updated))
            .collect();

        let mix: Vec<String> = candidates
            .iter()
            .filter_map(|t| t.template.clone())
            .collect();

        let stats = budget::BudgetStats::load(&self.vault_root);
        let (reserve_5h, reserve_7d) = stats.reserve_for(&mix, &store.config().budget);
        let rich_verdict = budget::verdict(
            snap.as_ref(),
            running_dispatcher,
            unreflected,
            &mix,
            &stats,
            &store.config().budget,
            now,
        );
        let verdict = to_plan_verdict(&rich_verdict);
        let budget_state = to_budget_state(
            &rich_verdict,
            snap.as_ref(),
            reserve_5h,
            reserve_7d,
            &store.config().budget,
            unreflected,
        );

        let catalog = self.catalog();
        let mut holders = self.worktree_holders(store);
        let mut manual_warnings = exempt_manual_starts(
            &candidates,
            &mut holders,
            store.config().dispatcher.max_worktrees_per_project,
        );
        let (dev_flows, test_flows) = lock(&self.flows)
            .as_ref()
            .map(|engine| engine.lane_load())
            .unwrap_or_default();
        let dispatch_plan = plan::plan_with_lanes(
            &candidates,
            &running_slots,
            &verdict,
            &catalog,
            &store.config().dispatcher,
            &holders,
            &plan::LaneLoad {
                routing: Some(store.routing()),
                dev_flows,
                test_flows,
            },
        );
        for launch in &dispatch_plan.launches {
            if let Some(detail) = &launch.detail {
                manual_warnings
                    .entry(launch.key.clone())
                    .or_insert_with(|| detail.as_str().to_string());
            }
        }

        BuiltPlan {
            dispatch_plan,
            running: state_running,
            updated_at: plan_updated_at,
            rich_verdict,
            budget_state,
            manual_warnings,
        }
    }

    /// Per service, the keys of tickets in progress, in review or blocked that own a live worktree.
    /// Only services with such a ticket are asked, so an idle project costs no git call.
    fn worktree_holders(&self, store: &TicketStore) -> HashMap<String, Vec<String>> {
        let mut candidates_by_service: HashMap<String, Vec<String>> = HashMap::new();
        for ticket in store.list(&ListFilter {
            status: vec![Status::InProgress, Status::InReview, Status::Blocked],
            ..ListFilter::default()
        }) {
            candidates_by_service
                .entry(ticket.service)
                .or_default()
                .push(ticket.key);
        }
        let mut holders = HashMap::new();
        for (service, keys) in candidates_by_service {
            let Some(project) = self.projects.get(&service) else {
                continue;
            };
            let live: HashSet<String> = worktrees::list(
                self.worktree_ops.runner.as_ref(),
                Path::new(&project.path),
                &self.vault_root,
            )
            .iter()
            .filter_map(|entry| worktrees::worktree_owner(&entry.path))
            .collect();
            let mut owning: Vec<String> =
                keys.into_iter().filter(|key| live.contains(key)).collect();
            owning.sort();
            holders.insert(service, owning);
        }
        holders
    }

    /// Dry run: builds the same plan a real tick would, launches nothing,
    /// writes nothing.
    pub fn plan_only(&self) -> Value {
        if self.vault_root.as_os_str().is_empty() {
            return json!({ FIELD_MODE: DispatcherMode::NoVault.as_str(), FIELD_LAUNCHES: [], FIELD_SKIPPED: [] });
        }
        let store = match TicketStore::open(&self.vault_root) {
            Ok(s) => s,
            Err(e) => return json!({ FIELD_ERROR: e.to_string() }),
        };
        if store.config().dispatcher.paused {
            return json!({
                FIELD_MODE: DispatcherMode::Paused.as_str(),
                FIELD_LAUNCHES: [],
                FIELD_SKIPPED: [],
                FIELD_SLOTS_USED: 0,
                FIELD_SLOTS_MAX: store.config().dispatcher.slots,
            });
        }
        let built = self.build_plan(&store);
        let mut v = serde_json::to_value(&built.dispatch_plan).unwrap_or_else(|_| json!({}));
        if let Some(obj) = v.as_object_mut() {
            obj.insert(
                "budget".to_string(),
                serde_json::to_value(&built.budget_state).unwrap_or_else(|_| json!({})),
            );
        }
        v
    }

    /// One dispatch cycle: never panics, never holds a lock across `start()`, never waits on another tick.
    /// It returns `{"mode": "busy"}` at once when a tick is in flight, and runs the GitHub label write-back and merge probe.
    pub fn tick(&self, reason: &str) -> Value {
        self.tick_inner(reason, true, None)
    }

    /// Same as [`Dispatcher::tick`] but skips the housekeeping and merge probe for this one call.
    /// For on-demand single-ticket starts, which must not block on network housekeeping.
    pub fn tick_local(&self, reason: &str) -> Value {
        self.tick_inner(reason, false, None)
    }

    /// Same as [`Dispatcher::tick_local`], carrying `key` as a force intent for that one ticket.
    /// It is inert outside a budget wait, and a separate method keeps the `tick` signatures unchanged.
    pub fn tick_local_forced(&self, reason: &str, key: &str) -> Value {
        self.tick_inner(reason, false, Some(key))
    }

    fn tick_inner(&self, reason: &str, run_housekeeping: bool, force_key: Option<&str>) -> Value {
        let _tick_guard = match self.tick_lock.try_lock() {
            Ok(guard) => guard,
            Err(TryLockError::WouldBlock) => return busy_report(reason),
            Err(TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
        };
        if self.vault_root.as_os_str().is_empty() {
            if !self.warned_no_vault.swap(true, Ordering::SeqCst) {
                warn!(
                    "no vault configured (AIDP_VAULT / ~/.aidp/config.json); every tick is a no-op until one is set"
                );
            }
            return json!({ FIELD_MODE: DispatcherMode::NoVault.as_str(), FIELD_LAST_REASON: reason, FIELD_LAUNCHED: [], FIELD_SKIPPED: [] });
        }

        let store = match TicketStore::open(&self.vault_root) {
            Ok(s) => s,
            Err(e) => {
                error!(
                    "cannot open the ticket store at {}: {e}",
                    self.vault_root.display()
                );
                return json!({ FIELD_ERROR: e.to_string() });
            }
        };

        crate::ticket_sync::after_tick(&self.vault_root);

        let cfg = store.config().dispatcher.clone();

        if let Some(key) = force_key {
            if let Some(report) = self.try_forced_during_wait(&store, reason, key) {
                return report;
            }
            if cfg.paused {
                if let Some(report) = self.try_forced_during_pause(&store, &cfg, reason, key) {
                    return report;
                }
            }
        }

        if let Some(report) = self.waiting_noop(&store, reason) {
            return report;
        }

        if cfg.paused {
            let state = DispatcherState {
                mode: DispatcherMode::Paused.as_str().to_string(),
                paused: true,
                wait_until: None,
                last_tick_at: now_iso(),
                last_reason: reason.to_string(),
                slots: SlotsState {
                    used: 0,
                    max: cfg.slots as usize,
                    test_used: 0,
                    test_max: cfg.test_slots as usize,
                },
                running: Vec::new(),
                last_plan: LastPlan::default(),
                budget: lock(&self.state).budget.clone(),
            };
            self.write_state(&store, &state);
            let report = json!({
                FIELD_MODE: DispatcherMode::Paused.as_str(),
                FIELD_LAUNCHED: Vec::<String>::new(),
                FIELD_SKIPPED: Vec::<SkippedEntry>::new(),
                FIELD_SLOTS_USED: 0,
                FIELD_SLOTS_MAX: state.slots.max,
            });
            self.bus.emit(EVENT_DISPATCHER_TICK, report.clone());
            return report;
        }

        if self.first_tick.swap(false, Ordering::SeqCst) {
            self.sweep_orphans(&store);
        }

        let tickets_synced: Option<Value> = None;
        if run_housekeeping {
            let due = match *lock(&self.last_sync) {
                None => true,
                Some(ts) => now_ms() - ts >= MERGE_PROBE_INTERVAL_MS,
            };
            if due {
                self.probe_reviews(&store);
                *lock(&self.last_sync) = Some(now_ms());
            }
            self.probe_native_runs(&store);
        }

        let previous_mode = lock(&self.state).mode.clone();
        let BuiltPlan {
            dispatch_plan,
            running: mut state_running,
            updated_at: plan_updated_at,
            rich_verdict,
            budget_state,
            manual_warnings,
        } = self.build_plan(&store);

        let is_waiting = matches!(budget_state.verdict.as_str(), VERDICT_WAIT | VERDICT_PAUSED);
        let wait_until = match &rich_verdict {
            budget::Verdict::Wait { until_ms, .. } => millis_to_rfc3339(*until_ms),
            _ => None,
        };

        if is_waiting && previous_mode != DispatcherMode::Waiting.as_str() {
            let (window, utilization, limit) = match &rich_verdict {
                budget::Verdict::Wait {
                    window,
                    utilization,
                    limit,
                    ..
                } => (Some(window_key(window).to_string()), *utilization, *limit),
                _ => (None, budget_state.utilization_5h, budget_state.limit_5h),
            };
            self.bus.emit(
                EVENT_DISPATCHER_WAITING,
                json!({
                    FIELD_UNTIL: wait_until,
                    FIELD_WINDOW: window,
                    FIELD_UTILIZATION: utilization,
                    FIELD_LIMIT: limit,
                }),
            );
        } else if !is_waiting && previous_mode == "waiting" {
            self.bus.emit(
                EVENT_DISPATCHER_RESUMED,
                json!({ FIELD_REASON: BUDGET_RESUME_REASON }),
            );
        }

        let mut launched: Vec<String> = Vec::new();
        let mut launched_sessions: Vec<LaunchedSession> = Vec::new();
        let mut skipped: Vec<SkippedEntry> = dispatch_plan
            .skipped
            .iter()
            .map(|(key, reason)| SkippedEntry {
                key: key.clone(),
                reason: SkipReasonWire::from(reason).to_string(),
                detail: reason.detail(),
            })
            .collect();

        for l in &dispatch_plan.launches {
            match self.execute_launch(&store, l, &plan_updated_at) {
                LaunchAttemptOutcome::Launched(session_id) => {
                    launched.push(l.key.clone());
                    launched_sessions.push(LaunchedSession {
                        key: l.key.clone(),
                        session_id,
                        service: l.service.clone(),
                    });
                }
                LaunchAttemptOutcome::Skipped { reason, detail } => skipped.push(SkippedEntry {
                    key: l.key.clone(),
                    reason: reason.to_string(),
                    detail,
                }),
            }
        }
        let launch_warnings =
            self.record_launch_warnings(&mut state_running, &launched_sessions, &manual_warnings);

        let state = DispatcherState {
            mode: if is_waiting {
                DispatcherMode::Waiting.as_str().to_string()
            } else {
                DispatcherMode::Normal.as_str().to_string()
            },
            paused: false,
            wait_until: if is_waiting { wait_until } else { None },
            last_tick_at: now_iso(),
            last_reason: reason.to_string(),
            slots: SlotsState {
                used: dispatch_plan.slots_used,
                max: dispatch_plan.slots_max,
                test_used: dispatch_plan.test_slots_used,
                test_max: dispatch_plan.test_slots_max,
            },
            running: state_running,
            last_plan: LastPlan {
                launched: launched.clone(),
                skipped: skipped.clone(),
            },
            budget: budget_state,
        };
        self.write_state(&store, &state);

        let report = with_warnings(
            json!({
                FIELD_LAUNCHED: launched,
                FIELD_SKIPPED: skipped,
                FIELD_SLOTS_USED: state.slots.used,
                FIELD_SLOTS_MAX: state.slots.max,
                FIELD_TICKETS_SYNCED: tickets_synced,
            }),
            &launch_warnings,
        );
        self.bus.emit(EVENT_DISPATCHER_TICK, report.clone());
        report
    }

    /// Attach each launched manual start's cap warning to its running entry, adding the entry
    /// when the plan-time running list predates the launch. Returns the warnings to report.
    fn record_launch_warnings(
        &self,
        running: &mut Vec<RunningEntry>,
        launched: &[LaunchedSession],
        warnings: &HashMap<String, String>,
    ) -> Vec<String> {
        let mut reported = Vec::new();
        for session in launched {
            let Some(warning) = warnings.get(&session.key) else {
                continue;
            };
            match running.iter_mut().find(|entry| entry.key == session.key) {
                Some(entry) => {
                    if !entry.warnings.contains(warning) {
                        entry.warnings.push(warning.clone());
                    }
                }
                None => running.push(RunningEntry {
                    key: session.key.clone(),
                    session_id: session.session_id.clone(),
                    project: session.service.clone(),
                    started_at: now_iso(),
                    warnings: vec![warning.clone()],
                }),
            }
            reported.push(warning.clone());
        }
        reported
    }

    /// The cap warning of a forced start of `ticket`, `None` when its project is under the cap.
    fn forced_cap_warning(
        &self,
        store: &TicketStore,
        ticket: &aidp_tickets::model::Ticket,
    ) -> Option<String> {
        let holders = self.worktree_holders(store);
        let service_holders = holders.get(&ticket.service).map(Vec::as_slice)?;
        manual_cap_warning(
            service_holders,
            store.config().dispatcher.max_worktrees_per_project,
            &ticket.key,
        )
    }

    /// Re-read `key`, build its launch and execute it under the `journal_reason` transition.
    /// The warning is the cap warning when the manual start goes over the worktree cap.
    fn run_forced_launch(
        &self,
        store: &TicketStore,
        key: &str,
        journal_reason: &str,
    ) -> ForcedLaunch {
        let ticket = match store.get(key) {
            Ok((ticket, _body, _extra)) => ticket,
            Err(e) => {
                error!("forced start: cannot re-read ticket {key}: {e}");
                return ForcedLaunch::without_session(skipped(aidp_tickets::SkipReason::Vanished));
            }
        };
        let catalog = self.catalog();
        let built = if ticket.autopilot.phase() == model::Phase::ToTest {
            plan::build_test_launch(&ticket, Some(store.routing()))
        } else {
            plan::build_launch(&ticket, &catalog)
        };
        let launch = match built {
            Ok(launch) => launch,
            Err(skip_reason) => {
                return ForcedLaunch::without_session(skipped(skip_reason.as_str()));
            }
        };
        let warning = self.forced_cap_warning(store, &ticket);
        let plan_updated_at: HashMap<String, DateTime<Utc>> =
            [(ticket.key.clone(), ticket.updated)].into_iter().collect();
        let outcome = self.execute_launch_reason(store, &launch, &plan_updated_at, journal_reason);
        let session = match &outcome {
            LaunchAttemptOutcome::Launched(session_id) => Some(LaunchedSession {
                key: key.to_string(),
                session_id: session_id.clone(),
                service: launch.service.clone(),
            }),
            LaunchAttemptOutcome::Skipped { .. } => None,
        };
        ForcedLaunch {
            outcome,
            session,
            warning,
        }
    }

    /// While a budget wait holds and its deadline has not passed, persist the state and return a no-op report.
    /// `None` when not waiting or the wait elapsed, so the caller runs a real tick.
    fn waiting_noop(&self, store: &TicketStore, reason: &str) -> Option<Value> {
        let prev = lock(&self.state).clone();
        if prev.mode != DispatcherMode::Waiting.as_str() {
            return None;
        }
        let until_ms = prev.wait_until.as_deref().and_then(parse_rfc3339_ms)?;
        if now_ms() >= until_ms {
            return None;
        }
        let state = DispatcherState {
            mode: DispatcherMode::Waiting.as_str().to_string(),
            paused: false,
            wait_until: prev.wait_until.clone(),
            last_tick_at: now_iso(),
            last_reason: reason.to_string(),
            slots: prev.slots.clone(),
            running: prev.running.clone(),
            last_plan: prev.last_plan.clone(),
            budget: prev.budget.clone(),
        };
        self.write_state(store, &state);
        let report = json!({
            FIELD_MODE: DispatcherMode::Waiting.as_str(),
            FIELD_LAUNCHED: Vec::<String>::new(),
            FIELD_SKIPPED: Vec::<SkippedEntry>::new(),
            FIELD_SLOTS_USED: state.slots.used,
            FIELD_SLOTS_MAX: state.slots.max,
        });
        self.bus.emit(EVENT_DISPATCHER_TICK, report.clone());
        Some(report)
    }

    /// Forced launch of `key` alone during a budget wait, journalled as `forced-over-budget`.
    /// The wait still holds for everyone else; `None` when not waiting so the normal flow runs.
    fn try_forced_during_wait(
        &self,
        store: &TicketStore,
        reason: &str,
        key: &str,
    ) -> Option<Value> {
        let prev = lock(&self.state).clone();
        if prev.mode != DispatcherMode::Waiting.as_str() {
            return None;
        }
        let until_ms = prev.wait_until.as_deref().and_then(parse_rfc3339_ms)?;
        if now_ms() >= until_ms {
            return None;
        }

        let ForcedLaunch {
            outcome,
            session,
            warning,
        } = self.run_forced_launch(store, key, FORCED_OVER_BUDGET_REASON);

        let (launched, skipped) = match outcome {
            LaunchAttemptOutcome::Launched(_session_id) => (vec![key.to_string()], Vec::new()),
            LaunchAttemptOutcome::Skipped { reason, detail } => (
                Vec::new(),
                vec![SkippedEntry {
                    key: key.to_string(),
                    reason: reason.to_string(),
                    detail,
                }],
            ),
        };

        let mut state_running = self.running_entries();
        let warnings = self.forced_launch_warnings(&mut state_running, session, warning);
        let state = DispatcherState {
            mode: DispatcherMode::Waiting.as_str().to_string(),
            paused: false,
            wait_until: prev.wait_until.clone(),
            last_tick_at: now_iso(),
            last_reason: reason.to_string(),
            slots: SlotsState {
                used: state_running.len(),
                max: prev.slots.max,
                test_used: prev.slots.test_used,
                test_max: prev.slots.test_max,
            },
            running: state_running,
            last_plan: LastPlan {
                launched: launched.clone(),
                skipped: skipped.clone(),
            },
            budget: prev.budget.clone(),
        };
        self.write_state(store, &state);
        let report = with_warnings(
            json!({
                FIELD_MODE: DispatcherMode::Waiting.as_str(),
                FIELD_LAUNCHED: launched,
                FIELD_SKIPPED: skipped,
                FIELD_SLOTS_USED: state.slots.used,
                FIELD_SLOTS_MAX: state.slots.max,
            }),
            &warnings,
        );
        self.bus.emit(EVENT_DISPATCHER_TICK, report.clone());
        Some(report)
    }

    /// Record the cap warning of a forced start that did launch, and return it for the report.
    fn forced_launch_warnings(
        &self,
        running: &mut Vec<RunningEntry>,
        session: Option<LaunchedSession>,
        warning: Option<String>,
    ) -> Vec<String> {
        let (Some(session), Some(warning)) = (session, warning) else {
            return Vec::new();
        };
        let warnings = HashMap::from([(session.key.clone(), warning)]);
        self.record_launch_warnings(running, &[session], &warnings)
    }

    /// Forced launch of `key` alone during a config pause, journalled as `forced-over-pause`.
    /// It still enforces the slot ceiling and leaves the per-project rule to `start()`.
    fn try_forced_during_pause(
        &self,
        store: &TicketStore,
        cfg: &aidp_tickets::config::DispatcherConfig,
        reason: &str,
        key: &str,
    ) -> Option<Value> {
        let running_records = self.sessions.list(Some(SessionStatus::Running), None, 0);
        let slots_used = running_records
            .iter()
            .filter(|rec| rec.origin.kind == DISPATCHER_ORIGIN_KIND)
            .count();
        let slots_max = cfg.slots as usize;

        let ForcedLaunch {
            outcome,
            session,
            warning,
        } = if slots_used >= slots_max {
            ForcedLaunch::without_session(skipped(NO_SLOT_REASON))
        } else {
            self.run_forced_launch(store, key, FORCED_OVER_PAUSE_REASON)
        };

        let (launched, skipped) = match outcome {
            LaunchAttemptOutcome::Launched(_session_id) => (vec![key.to_string()], Vec::new()),
            LaunchAttemptOutcome::Skipped { reason, detail } => (
                Vec::new(),
                vec![SkippedEntry {
                    key: key.to_string(),
                    reason: reason.to_string(),
                    detail,
                }],
            ),
        };

        let mut state_running = self.running_entries();
        let warnings = self.forced_launch_warnings(&mut state_running, session, warning);
        let state = DispatcherState {
            mode: DispatcherMode::Paused.as_str().to_string(),
            paused: true,
            wait_until: None,
            last_tick_at: now_iso(),
            last_reason: reason.to_string(),
            slots: SlotsState {
                used: state_running.len(),
                max: slots_max,
                test_used: 0,
                test_max: cfg.test_slots as usize,
            },
            running: state_running,
            last_plan: LastPlan {
                launched: launched.clone(),
                skipped: skipped.clone(),
            },
            budget: lock(&self.state).budget.clone(),
        };
        self.write_state(store, &state);
        let report = with_warnings(
            json!({
                FIELD_MODE: DispatcherMode::Paused.as_str(),
                FIELD_LAUNCHED: launched,
                FIELD_SKIPPED: skipped,
                FIELD_SLOTS_USED: state.slots.used,
                FIELD_SLOTS_MAX: state.slots.max,
            }),
            &warnings,
        );
        self.bus.emit(EVENT_DISPATCHER_TICK, report.clone());
        Some(report)
    }

    /// The dispatcher-owned `RunningEntry` rows for `dispatcher.json`, read fresh from running sessions.
    /// Shared so the forced paths need not rebuild the whole plan.
    fn running_entries(&self) -> Vec<RunningEntry> {
        Self::running_entries_from(&self.sessions.list(Some(SessionStatus::Running), None, 0))
    }

    fn running_entries_from(
        running_records: &[crate::sessions::SessionRecord],
    ) -> Vec<RunningEntry> {
        running_records
            .iter()
            .filter_map(|rec| {
                let key = dispatcher_ticket_key(&rec.origin)?.to_string();
                Some(RunningEntry {
                    key,
                    session_id: rec.id.clone(),
                    project: rec.project_name.clone(),
                    started_at: rec.started_at.clone(),
                    warnings: Vec::new(),
                })
            })
            .collect()
    }

    fn write_state(&self, store: &TicketStore, state: &DispatcherState) {
        *lock(&self.state) = state.clone();
        let path = dispatcher_state_path(store.vault_root());
        if let Err(e) = crate::store::write_json(&path, state) {
            error!("cannot write {}: {e}", path.display());
        }
    }

    pub fn state_json(&self) -> Value {
        serde_json::to_value(&*lock(&self.state)).unwrap_or_else(|_| json!({}))
    }

    /// Budget explainer for `GET /api/dispatcher/budget`: persisted verdict, live windows, reserves, stats.
    /// The verdict is never recomputed here, so it cannot drift from the tick's decision.
    pub fn budget_json(&self) -> Value {
        let persisted = lock(&self.state).budget.clone();

        let live = self.sessions.rate_limit();
        let live_json = live.as_ref().map(|info| {
            let window = |w: &crate::sessions::RateLimitWindow| {
                json!({
                    FIELD_UTILIZATION: w.utilization,
                    FIELD_RESETS_AT: millis_to_rfc3339(w.resets_at * 1000),
                })
            };
            json!({
                FIELD_STATUS: info.status,
                FIELD_IS_USING_OVERAGE: info.is_using_overage,
                FIELD_FIVE_HOUR: info.five_hour.as_ref().map(window),
                FIELD_SEVEN_DAY: info.seven_day.as_ref().map(window),
                FIELD_OBSERVED_AT: millis_to_rfc3339(info.observed_at),
            })
        });

        let budget_cfg = if self.vault_root.as_os_str().is_empty() {
            aidp_tickets::config::BudgetConfig::default()
        } else {
            aidp_tickets::config::load(&self.vault_root).budget
        };
        let stats = budget::BudgetStats::load(&self.vault_root);

        json!({
            FIELD_VERDICT: serde_json::to_value(&persisted).unwrap_or_else(|_| json!({})),
            FIELD_LIVE: live_json,
            FIELD_RESERVES: {
                FIELD_FIVE_HOUR: persisted.reserve_5h,
                FIELD_SEVEN_DAY: persisted.reserve_7d,
                FIELD_LEARNED: budget_cfg.learn,
            },
            FIELD_UNREFLECTED: persisted.unreflected,
            FIELD_STATS: serde_json::to_value(&stats).unwrap_or_else(|_| json!({})),
        })
    }

    /// Set `dispatcher.paused` in `TODO/config.yaml`, then refresh the cache and emit.
    /// The leaf is patched in place so comments, blank lines and key order survive; the whole `Config`
    /// is re-serialized only when the file is missing or its shape is out of reach of the patch.
    pub fn set_paused(&self, paused: bool, actor: &str) -> anyhow::Result<bool> {
        if self.vault_root.as_os_str().is_empty() {
            anyhow::bail!("cannot change the pause state: no vault configured");
        }
        let mut state = lock(&self.state);
        let mut cfg = aidp_tickets::config::load(&self.vault_root);
        if cfg.dispatcher.paused == paused {
            return Ok(paused);
        }
        cfg.dispatcher.paused = paused;
        let path = self.vault_root.join(TODO_DIR).join(CONFIG_FILE);
        let text = match patched_paused_text(&path, paused) {
            Some(text) => text,
            None => serde_yaml::to_string(&cfg)?,
        };
        write_text_atomic(&path, &text)?;
        state.paused = paused;
        state.mode = if paused {
            DispatcherMode::Paused
        } else {
            DispatcherMode::Normal
        }
        .as_str()
        .to_string();
        let event = DispatcherPausedChangedEvent {
            paused,
            actor: actor.to_string(),
            at: now_iso(),
        };
        self.bus.emit(
            EVENT_DISPATCHER_PAUSED_CHANGED,
            serde_json::to_value(event)?,
        );
        Ok(paused)
    }

    /// PR search order: `pr/pr.json`, then `outputs.pr_url` in `run-state.json`, then `changes::pr_for_branch`.
    /// Only the last shells out to `gh`, so never call it on a per-tick path.
    fn find_pr(
        &self,
        cwd: &str,
        workflow_run_id: Option<&str>,
        key: &str,
    ) -> Option<lifecycle::PrInfo> {
        let run_dir = workflow_run_id.and_then(|run_id| self.run_dir_of(cwd, run_id));
        if let Some(dir) = &run_dir {
            if let Some(pr) = read_pr_json(&dir.join(PR_DIR).join(PR_FILE)) {
                return Some(pr);
            }
            if let Some(pr) = read_pr_url_from_run_state(dir) {
                return Some(pr);
            }
        }
        let branch = feature_branch(key);
        crate::changes::pr_for_branch(Path::new(cwd), &branch).map(|v| json_value_to_pr_info(&v))
    }

    /// One `Launch` from the plan: re-read, project resolution, queued transition, inputs file, `start()`.
    /// The outcome is then reconciled.
    fn execute_launch(
        &self,
        store: &TicketStore,
        l: &plan::Launch,
        plan_updated_at: &HashMap<String, DateTime<Utc>>,
    ) -> LaunchAttemptOutcome {
        self.execute_launch_reason(store, l, plan_updated_at, DISPATCH_REASON)
    }

    /// Same as [`Dispatcher::execute_launch`] but journals `reason` on the queued transition.
    /// Forced launches pass `forced-over-budget` or `forced-over-pause` to stay distinguishable.
    #[tracing::instrument(
        skip_all,
        fields(run_id = tracing::field::Empty, ticket_key = %l.key, session_id = tracing::field::Empty)
    )]
    fn execute_launch_reason(
        &self,
        store: &TicketStore,
        l: &plan::Launch,
        plan_updated_at: &HashMap<String, DateTime<Utc>>,
        reason: &str,
    ) -> LaunchAttemptOutcome {
        let (mut ticket, body, extra) = match store.get(&l.key) {
            Ok(v) => v,
            Err(e) => {
                error!("cannot re-read ticket {}: {e}", l.key);
                return skipped(aidp_tickets::SkipReason::Vanished);
            }
        };
        if plan_updated_at.get(&l.key) != Some(&ticket.updated) {
            return skipped(aidp_tickets::SkipReason::ChangedSincePlan);
        }

        let Some(project) = self.projects.get(&l.service) else {
            return skipped_with_detail(
                aidp_tickets::SkipReason::NeedsRouting,
                format!(
                    "no project named `{}` — run `aidp ticket route {} <service>`",
                    l.service, l.key
                ),
            );
        };

        // A flow owns its PR, and a manual or forced start means "run it": neither is
        // short-circuited by an open PR (the issue stays in its lane until `to-test`).
        let adoptable = l.flow.is_none()
            && !l.manual
            && reason == DISPATCH_REASON
            && store.config().dispatcher.launch != DispatcherLaunch::Flow;
        if adoptable {
            if let Some(outcome) =
                self.adopt_existing_pr(store, &mut ticket, &body, &extra, &project.path)
            {
                return outcome;
            }
        }

        if l.flow.is_some() || store.config().dispatcher.launch == DispatcherLaunch::Flow {
            return self.start_delivery_flow(store, l);
        }

        if self.native_workspace_busy(&project.path, &l.key) {
            return skipped(aidp_tickets::SkipReason::WorkspaceBusy);
        }

        let queued_from = ticket.status;
        if queued_from != Status::Queued {
            match model::guard(queued_from, Status::Queued, Actor::Dispatcher, false) {
                Ok(()) => ticket.status = Status::Queued,
                Err(e) => {
                    error!("cannot queue {}: {e}", l.key);
                    return skipped(aidp_tickets::SkipReason::IllegalTransition);
                }
            }
        }
        let queued_event = Event {
            ts: SystemClock.now_utc(),
            key: ticket.key.clone(),
            from: Some(queued_from),
            to: Status::Queued,
            actor: Actor::Dispatcher,
            reason: Some(reason.to_string()),
            session_id: None,
        };
        if let Err(e) = store.save(&mut ticket, &body, &extra) {
            error!("cannot save {} as queued: {e}", l.key);
            return skipped(aidp_tickets::SkipReason::SaveFailed);
        }
        ledger::append(store.vault_root(), &queued_event);

        let inputs_value = match &l.demand {
            plan::Demand::Issue { number } => {
                json!({ INPUT_ISSUE: number.to_string(), INPUT_TRACK: l.track })
            }
            plan::Demand::Prompt { feature, .. } => {
                json!({ INPUT_PROMPT: body, INPUT_FEATURE: feature, INPUT_TRACK: l.track })
            }
        };
        let inputs_path = store
            .vault_root()
            .join(TODO_DIR)
            .join(STATE_DIR)
            .join(LAUNCH_DIR)
            .join(format!("{}{INPUTS_FILE_SUFFIX}", l.key));
        if let Err(e) = write_inputs_file(&inputs_path, &inputs_value) {
            error!("cannot write inputs file for {}: {e}", l.key);
            return skipped(aidp_tickets::SkipReason::InputsFileFailed);
        }

        let req = dispatcher_start_request(
            &project.id,
            l,
            &ticket.summary,
            &inputs_path,
            &store.config().dispatcher,
            None,
            None,
        );

        let started = match self.launch_run(req) {
            Ok(launch) => launch,
            Err(e) => {
                error!("start() failed for {}: {e}", l.key);
                return start_failure_outcome(&e);
            }
        };

        let launch = match started {
            LaunchedRun::Native { run_id } => {
                return self.record_started(
                    store,
                    (&mut ticket, &body, &extra),
                    l,
                    &run_id,
                    Some(run_id.clone()),
                );
            }
            LaunchedRun::Session(launch) => launch,
        };

        match reconcile_launch(&launch, |id| {
            self.sessions.queue_cancel(id);
        }) {
            LaunchOutcome::Started(session_id) => {
                self.record_started(store, (&mut ticket, &body, &extra), l, &session_id, None)
            }
            LaunchOutcome::Requeued(queue_id) => {
                let from = ticket.status;
                if let Some(event) = guard_and_build_event(
                    "cannot requeue after queue-race",
                    &ticket.key,
                    from,
                    Status::Todo,
                    format!("{QUEUE_RACE_REASON}:{queue_id}"),
                    None,
                ) {
                    ticket.status = Status::Todo;
                    if let Err(e) = store.save(&mut ticket, &body, &extra) {
                        error!("cannot save {} after queue-race: {e}", l.key);
                    }
                    ledger::append(store.vault_root(), &event);
                }
                skipped(aidp_tickets::SkipReason::QueueRace)
            }
        }
    }

    /// Starts the default flow of the ticket instead of a session, then moves it to `in-progress`.
    /// A ticket with an active flow run is skipped, never started twice.
    fn start_delivery_flow(&self, store: &TicketStore, l: &plan::Launch) -> LaunchAttemptOutcome {
        let Some(engine) = lock(&self.flows).clone() else {
            return skipped(aidp_tickets::SkipReason::FlowUnavailable);
        };
        let Ok((ticket, _, _)) = store.get(&l.key) else {
            return skipped(aidp_tickets::SkipReason::Vanished);
        };
        let ticket_value = match serde_json::to_value(&ticket) {
            Ok(value) => value,
            Err(e) => return start_failure_outcome(&e.into()),
        };
        match engine.start(ticket_value, l.flow.as_deref(), None) {
            Ok(state) => self.record_flow_started(store, l, &state.def.id),
            Err(e) => match e.downcast_ref::<FlowEngineError>() {
                Some(FlowEngineError::Conflict { .. }) => {
                    skipped(aidp_tickets::SkipReason::FlowActive)
                }
                _ => {
                    error!("flow start failed for {}: {e}", l.key);
                    start_failure_outcome(&e)
                }
            },
        }
    }

    /// Re-reads the ticket the flow may already have touched, then marks it driven by `flow_id`.
    fn record_flow_started(
        &self,
        store: &TicketStore,
        l: &plan::Launch,
        flow_id: &str,
    ) -> LaunchAttemptOutcome {
        let flow_ref = format!("{FLOW_SESSION_PREFIX}{}", l.key);
        let Ok((mut ticket, body, extra)) = store.get(&l.key) else {
            return skipped(aidp_tickets::SkipReason::Vanished);
        };
        let from = ticket.status;
        let moves = from != Status::InProgress;
        if moves {
            if let Err(e) = model::guard(from, Status::InProgress, Actor::Flow, false) {
                warn!("cannot start {} under a flow: {e}", l.key);
                return LaunchAttemptOutcome::Launched(flow_ref);
            }
        }
        ticket.status = Status::InProgress;
        ticket.autopilot.flow = Some(flow_id.to_string());
        ticket.autopilot.set_phase(
            if l.test {
                model::Phase::Testing
            } else {
                model::Phase::Dev
            },
            SystemClock.now_utc(),
        );
        ticket.autopilot.branch = Some(branch_for_ticket(ticket.r#type, &l.key));
        ticket.autopilot.started_at = Some(SystemClock.now_utc());
        ticket.autopilot.blocked_reason = None;
        if let Err(e) = store.save(&mut ticket, &body, &extra) {
            error!("cannot save {} as flow-started: {e}", l.key);
        }
        if moves {
            ledger::append(
                store.vault_root(),
                &Event {
                    ts: SystemClock.now_utc(),
                    key: ticket.key.clone(),
                    from: Some(from),
                    to: Status::InProgress,
                    actor: Actor::Flow,
                    reason: Some(FlowTicketReason::Started.as_str().to_string()),
                    session_id: None,
                },
            );
        }
        emit_ticket_status(&self.bus, &ticket.key, ticket.status.as_str());
        LaunchAttemptOutcome::Launched(flow_ref)
    }

    /// Starts `req` through the launcher when there is one, else as a legacy session.
    fn launch_run(&self, req: StartRequest) -> anyhow::Result<LaunchedRun> {
        match &self.launcher {
            Some(launcher) => launcher.launch(req, true),
            None => Ok(LaunchedRun::Session(self.sessions.start(req)?)),
        }
    }

    /// True when a session or a live native run already holds the ticket's workspace.
    fn native_workspace_busy(&self, project_path: &str, key: &str) -> bool {
        let Some(launcher) = &self.launcher else {
            return false;
        };
        let repo = aidp_core::vault::repo_segment(Path::new(project_path));
        let workspace = crate::flows::lock::workspace_key(&self.vault_root, &repo, key);
        self.sessions.is_workspace_busy(&workspace)
            || launcher.native().list().iter().any(|view| {
                is_live_native(view.status) && view.request.ticket.as_deref() == Some(key)
            })
    }

    /// Moves the ticket to `in-progress` for the launched `session_id`, with `run_id` set for a native run.
    fn record_started(
        &self,
        store: &TicketStore,
        (ticket, body, extra): (&mut aidp_tickets::model::Ticket, &str, &serde_yaml::Mapping),
        l: &plan::Launch,
        session_id: &str,
        run_id: Option<String>,
    ) -> LaunchAttemptOutcome {
        tracing::Span::current().record(crate::observability::FIELD_SESSION_ID, session_id);
        info!(ticket_key = %l.key, session_id = %session_id, "session launched");
        let event = lifecycle::on_started(ticket, session_id, run_id, &feature_branch(&l.key));
        if let Err(e) = store.save(ticket, body, extra) {
            error!("cannot save {} as started: {e}", l.key);
        }
        ledger::append(store.vault_root(), &event);
        let started = TicketStartedEvent {
            key: &l.key,
            session_id,
            project: &l.service,
        };
        match serde_json::to_value(&started) {
            Ok(payload) => {
                self.bus.emit(EVENT_TICKET_STARTED, payload);
            }
            Err(e) => error!("cannot serialize ticket.started for {}: {e}", l.key),
        }
        LaunchAttemptOutcome::Launched(session_id.to_string())
    }

    /// The vault run directory of `run_id`: the native manager's, else found from `cwd`.
    fn run_dir_of(&self, cwd: &str, run_id: &str) -> Option<PathBuf> {
        self.launcher
            .as_ref()
            .and_then(|launcher| launcher.native().get(run_id))
            .map(|view| view.run_dir)
            .or_else(|| crate::runs::resolve_run(cwd, Some(run_id)).map(|run| run.run_dir))
    }

    /// Ends the `in-progress` tickets whose native run stopped, with the PR the run opened.
    fn probe_native_runs(&self, store: &TicketStore) {
        let Some(launcher) = &self.launcher else {
            return;
        };
        for candidate in store.list(&ListFilter {
            status: vec![Status::InProgress],
            ..ListFilter::default()
        }) {
            let Some(run_id) = candidate.autopilot.run_id.clone() else {
                continue;
            };
            let Some(outcome) = launcher
                .native()
                .get(&run_id)
                .and_then(|view| native_outcome(view.status))
            else {
                continue;
            };
            let Some(project) = self.projects.get(&candidate.service) else {
                continue;
            };
            let Ok((mut ticket, body, extra)) = store.get(&candidate.key) else {
                continue;
            };
            let pr = self.find_pr(&project.path, Some(&run_id), &ticket.key);
            let cfg = store.config().dispatcher.clone();
            let event = lifecycle::on_ended(&mut ticket, outcome, pr.clone(), &cfg);
            if let Err(e) = store.save(&mut ticket, &body, &extra) {
                error!("cannot save {} after native run end: {e}", ticket.key);
            }
            ledger::append(store.vault_root(), &event);
            let status = TicketStatusEvent {
                key: &ticket.key,
                status: ticket.status.as_str(),
            };
            match serde_json::to_value(&status) {
                Ok(payload) => {
                    self.bus.emit(EVENT_TICKET_STATUS, payload);
                }
                Err(e) => error!("cannot serialize ticket.status for {}: {e}", ticket.key),
            }
        }
    }

    /// Adopts the pull request `ticket` already has instead of launching a session.
    /// An open one moves it to `in-review`, a merged one to `done`; closed unmerged is ignored.
    fn adopt_existing_pr(
        &self,
        store: &TicketStore,
        ticket: &mut aidp_tickets::model::Ticket,
        body: &str,
        extra: &serde_yaml::Mapping,
        cwd: &str,
    ) -> Option<LaunchAttemptOutcome> {
        let candidate = crate::pr_adoption::find_for_ticket(Path::new(cwd), &ticket.key)?;
        let event = lifecycle::adopt(
            ticket,
            candidate.adopted.clone(),
            Actor::Dispatcher,
            SystemClock.now_utc(),
        )
        .map_err(|e| error!("cannot adopt PR for {}: {e}", ticket.key))
        .ok()?;
        if let Err(e) = self.persist_adoption(store, ticket, body, extra, &event) {
            error!("cannot save {} after adoption: {e}", ticket.key);
            return Some(skipped(aidp_tickets::SkipReason::SaveFailed));
        }
        let reason = event.reason.clone().unwrap_or_default();
        Some(skipped_with_detail(reason, candidate.adopted.pr.url))
    }

    /// Saves an adopted ticket, journals the transition and announces it.
    fn persist_adoption(
        &self,
        store: &TicketStore,
        ticket: &mut aidp_tickets::model::Ticket,
        body: &str,
        extra: &serde_yaml::Mapping,
        event: &Event,
    ) -> anyhow::Result<()> {
        store.save(ticket, body, extra)?;
        ledger::append(store.vault_root(), event);
        let payload = TicketStatusEvent {
            key: &ticket.key,
            status: ticket.status.as_str(),
        };
        self.bus
            .emit(EVENT_TICKET_STATUS, serde_json::to_value(payload)?);
        Ok(())
    }

    /// Adopts the pull request `reference` (url or number) for `key` as a human.
    /// An unknown ticket, project or pull request is `NotFound`; a closed or illegal one is `Conflict`.
    pub fn adopt_ticket(
        &self,
        store: &TicketStore,
        key: &str,
        reference: &str,
    ) -> anyhow::Result<AdoptResponse> {
        let (mut ticket, body, extra) = store
            .get(key)
            .map_err(|e| anyhow::Error::new(SessionError::NotFound(e.to_string())))?;
        let project = self.projects.get(&ticket.service).ok_or_else(|| {
            SessionError::NotFound(format!("no project named `{}`", ticket.service))
        })?;
        let view =
            crate::pr_adoption::view_pr(Path::new(&project.path), reference).ok_or_else(|| {
                SessionError::NotFound(format!("pull request `{reference}` not found"))
            })?;
        let candidate = match crate::pr_adoption::classify_view(&view, &feature_branch(key)) {
            PrClassification::Adoptable(candidate) => candidate,
            PrClassification::ClosedUnmerged => {
                return Err(SessionError::Conflict(format!(
                    "pull request `{reference}` is closed without being merged"
                ))
                .into())
            }
        };
        let event = lifecycle::adopt(
            &mut ticket,
            candidate.adopted.clone(),
            Actor::Human,
            SystemClock.now_utc(),
        )
        .map_err(|e| SessionError::Conflict(e.to_string()))?;
        self.persist_adoption(store, &mut ticket, &body, &extra, &event)?;
        Ok(AdoptResponse { ticket, body })
    }

    /// On the first non-paused tick: orphan every `in-progress` ticket whose session is not running.
    /// Such tickets were put in progress by this dispatcher alone, so no origin check is needed.
    fn sweep_orphans(&self, store: &TicketStore) {
        let running_ids: HashSet<String> = self
            .sessions
            .list(Some(SessionStatus::Running), None, 0)
            .into_iter()
            .map(|r| r.id)
            .chain(self.live_native_run_ids())
            .collect();

        let in_progress = store.list(&ListFilter {
            status: vec![Status::InProgress],
            ..ListFilter::default()
        });

        for candidate in in_progress {
            let Some(session_id) = candidate.autopilot.session_id.clone() else {
                continue;
            };
            if running_ids.contains(&session_id) {
                continue;
            }
            let Ok((mut ticket, body, extra)) = store.get(&candidate.key) else {
                continue;
            };
            let cwd = self.projects.get(&ticket.service).map(|p| p.path.clone());
            let workflow_run_id = self
                .sessions
                .get(&session_id)
                .and_then(|record| record.workflow_run_id)
                .or_else(|| ticket.autopilot.run_id.clone());
            let pr = cwd
                .as_ref()
                .and_then(|cwd| self.find_pr(cwd, workflow_run_id.as_deref(), &ticket.key));
            let cfg = store.config().dispatcher.clone();
            let event =
                lifecycle::on_ended(&mut ticket, lifecycle::SessionOutcome::Orphaned, pr, &cfg);
            if let Err(e) = store.save(&mut ticket, &body, &extra) {
                error!("cannot save {} during orphan sweep: {e}", ticket.key);
            }
            ledger::append(store.vault_root(), &event);
            emit_ticket_status(&self.bus, &ticket.key, ticket.status.as_str());
        }
    }

    /// The ids of the native runs that are still executing or waiting on a human.
    fn live_native_run_ids(&self) -> Vec<String> {
        self.launcher
            .as_ref()
            .map(|launcher| {
                launcher
                    .native()
                    .list()
                    .into_iter()
                    .filter(|view| is_live_native(view.status))
                    .map(|view| view.run_id)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Move `in-review` tickets with a PR number to merged or closed as `gh pr view` reports.
    /// It shells out to `gh` once per ticket, so it only runs on the housekeeping tick.
    fn probe_reviews(&self, store: &TicketStore) {
        let in_review = store.list(&ListFilter {
            status: vec![Status::InReview],
            ..ListFilter::default()
        });

        for candidate in in_review {
            let recorded_number = candidate.autopilot.pr.as_ref().and_then(|pr| pr.number);
            let flow_owned = candidate.autopilot.flow.is_some();
            if recorded_number.is_none() && !flow_owned {
                continue;
            }
            let Some(project) = self.projects.get(&candidate.service) else {
                continue;
            };
            let references = merge_probe_references(&candidate.key, recorded_number, flow_owned);
            let Some(view) = first_pr_view(&references, |reference| {
                crate::changes::pr_for_branch(Path::new(&project.path), reference)
            }) else {
                continue;
            };
            let Some(pr_state) = final_pr_state(&view) else {
                continue;
            };
            if flow_owned {
                if matches!(pr_state, lifecycle::PrState::Merged) {
                    self.feed_flow_merge(&candidate, &view);
                }
                continue;
            }

            let Ok((mut ticket, body, extra)) = store.get(&candidate.key) else {
                continue;
            };
            if ticket.status == Status::Done {
                continue;
            }
            let Some(event) = lifecycle::on_pr_state(&mut ticket, pr_state) else {
                continue;
            };
            if let Err(e) = store.save(&mut ticket, &body, &extra) {
                error!("cannot save {} after pr state change: {e}", ticket.key);
                continue;
            }
            ledger::append(store.vault_root(), &event);
            emit_ticket_status(&self.bus, &ticket.key, ticket.status.as_str());
            if matches!(pr_state, lifecycle::PrState::Merged) {
                self.remove_ticket_worktree(&ticket.key, &ticket.service);
            }
        }
    }

    /// Announces the merge of a flow-owned ticket's PR; the flow's merge step moves the ticket itself.
    fn feed_flow_merge(&self, ticket: &model::Ticket, view: &Value) {
        let number = view
            .get(NUMBER_KEY)
            .and_then(Value::as_u64)
            .or_else(|| ticket.autopilot.pr.as_ref().and_then(|pr| pr.number));
        let Some(number) = number else {
            return;
        };
        let body = ActionBody {
            repo: ticket.service.clone(),
            number,
            result: InboxChangedEvent {},
            ticket: Some(ticket.key.clone()),
        };
        match serde_json::to_value(&body) {
            Ok(payload) => {
                self.bus.emit(EVENT_PR_MERGED, payload);
            }
            Err(e) => error!("cannot serialize pr.merged for {}: {e}", ticket.key),
        }
    }

    /// The shared opening of the session bus handlers: vault, `sessionId`, session, origin, store, ticket.
    /// Returns `None` at the first failing check; only dispatcher-owned ticket sessions are adopted.
    fn resolve_bus_event(&self, data: &Value) -> Option<ResolvedBusEvent> {
        if self.vault_root.as_os_str().is_empty() {
            return None;
        }
        let session_id = data[FIELD_SESSION_ID].as_str()?;
        let rec = self.sessions.get(session_id)?;
        let key = dispatcher_ticket_key(&rec.origin)?.to_string();
        let store = TicketStore::open(&self.vault_root).ok()?;
        let (ticket, body, extra) = store.get(&key).ok()?;
        Some(ResolvedBusEvent {
            store,
            ticket,
            body,
            extra,
            key,
            rec,
        })
    }

    fn on_bus_event(&self, ev: &Value) {
        let ty = ev[FIELD_TYPE].as_str().unwrap_or("");
        let data = &ev[FIELD_DATA];
        match ty {
            EVENT_SESSION_ENDED => self.on_session_ended(data),
            EVENT_SESSION_QUESTION => self.on_session_question(data),
            EVENT_SESSION_ANSWERED => self.on_session_answered(data),
            EVENT_PR_MERGED => self.on_pr_merged(data),
            EVENT_TICKET_STATUS => self.on_ticket_status(data),
            _ => {}
        }
    }

    /// A merged PR frees its ticket's worktree; the ticket is named by the event's `key`.
    fn on_pr_merged(&self, data: &Value) {
        if let Some(key) = data[FIELD_KEY].as_str() {
            self.release_worktree_of(key, None);
        }
    }

    /// A cancelled ticket frees its worktree. Other transitions leave it alone, so a ticket
    /// blocked by a closed PR keeps its worktree for the PR to be reopened.
    fn on_ticket_status(&self, data: &Value) {
        if data[FIELD_STATUS].as_str() == Some(Status::Cancelled.as_str()) {
            if let Some(key) = data[FIELD_KEY].as_str() {
                self.release_worktree_of(key, Some(Status::Cancelled));
            }
        }
    }

    /// Save then delete the worktree of ticket `key`; a `required` status guards against stale events.
    pub fn release_worktree_of(&self, key: &str, required: Option<Status>) {
        if self.vault_root.as_os_str().is_empty() {
            return;
        }
        let Ok(store) = TicketStore::open(&self.vault_root) else {
            return;
        };
        let Ok((ticket, _body, _extra)) = store.get(key) else {
            return;
        };
        if required.is_some_and(|status| status != ticket.status) {
            return;
        }
        self.remove_ticket_worktree(&ticket.key, &ticket.service);
    }

    /// Save, then delete, the worktree of `key` in the project registered as `service`.
    fn remove_ticket_worktree(&self, key: &str, service: &str) -> WorktreeRemoval {
        let Some(project) = self.projects.get(service) else {
            return WorktreeRemoval::Absent;
        };
        let project_path = Path::new(&project.path);
        let repo = aidp_core::vault::repo_segment(project_path);
        let path = worktrees::worktree_path(&self.vault_root, &repo, key);
        if !path.is_dir() {
            return WorktreeRemoval::Absent;
        }
        self.remove_worktree_at(project_path, &path, key)
    }

    /// Delete one worktree through [`worktrees::remove`], publishing the outcome and updating health.
    fn remove_worktree_at(&self, project_path: &Path, path: &Path, key: &str) -> WorktreeRemoval {
        let result = worktrees::remove(
            self.worktree_ops.runner.as_ref(),
            self.worktree_ops.sleeper.as_ref(),
            project_path,
            path,
            key,
        );
        match result {
            Ok(removed) => {
                health::clear_worktree_remove_failure(key);
                let mut payload = json!({ FIELD_KEY: key });
                if let Some(reference) = &removed.salvaged_ref {
                    payload[FIELD_SALVAGED_REF] = json!(reference);
                }
                self.bus.emit(EVENT_WORKTREE_REMOVED, payload);
                WorktreeRemoval::Removed(removed.salvaged_ref)
            }
            Err(error) => {
                let failure = remove_failure(path, key, &error);
                self.report_remove_failure(&failure);
                WorktreeRemoval::Failed(failure)
            }
        }
    }

    /// Record a failed deletion for `GET /api/health` and publish `worktree.remove_failed`.
    fn report_remove_failure(&self, failure: &WorktreeRemoveFailure) {
        warn!(
            ticket_key = %failure.key,
            "worktree removal failed: {}",
            failure.error
        );
        health::record_worktree_remove_failure(failure.clone());
        self.bus.emit(
            EVENT_WORKTREE_REMOVE_FAILED,
            json!({
                FIELD_KEY: failure.key,
                FIELD_PATH: failure.path,
                FIELD_ERROR: failure.error,
                FIELD_ATTEMPTS: failure.attempts,
            }),
        );
    }

    /// Save then delete every worktree whose ticket is done, cancelled or unknown.
    /// Failures stay in the health payload and are retried by the next sweep.
    pub fn sweep_worktrees(&self) -> WorktreeSweepReport {
        let mut report = WorktreeSweepReport::default();
        if self.vault_root.as_os_str().is_empty() {
            return report;
        }
        let Ok(store) = TicketStore::open(&self.vault_root) else {
            return report;
        };
        let statuses: HashMap<String, Status> = store
            .list(&ListFilter {
                archived: true,
                ..ListFilter::default()
            })
            .into_iter()
            .map(|ticket| (ticket.key, ticket.status))
            .collect();
        let projects = self.projects.list();
        for (repo, key, path) in worktree_dirs(&self.vault_root) {
            if !matches!(
                statuses.get(&key),
                None | Some(Status::Done) | Some(Status::Cancelled)
            ) {
                continue;
            }
            let Some(project) = projects
                .iter()
                .find(|project| aidp_core::vault::repo_segment(Path::new(&project.path)) == repo)
            else {
                let failure = WorktreeRemoveFailure {
                    key,
                    path: path.to_string_lossy().to_string(),
                    error: format!("{NO_PROJECT_FOR_REPO} {repo}"),
                    attempts: WORKTREE_MISSING_ATTEMPTS,
                };
                self.report_remove_failure(&failure);
                report.failed.push(failure);
                continue;
            };
            match self.remove_worktree_at(Path::new(&project.path), &path, &key) {
                WorktreeRemoval::Removed(salvaged_ref) => {
                    report.removed.push(SweptWorktree { key, salvaged_ref })
                }
                WorktreeRemoval::Failed(failure) => report.failed.push(failure),
                WorktreeRemoval::Absent => {}
            }
        }
        report
    }

    fn on_session_ended(&self, data: &Value) {
        let Some(ResolvedBusEvent {
            store,
            mut ticket,
            body,
            extra,
            key,
            rec,
        }) = self.resolve_bus_event(data)
        else {
            return;
        };

        let pr = self.find_pr(&rec.cwd, rec.workflow_run_id.as_deref(), &key);
        let outcome = match rec.status {
            SessionStatus::Done => lifecycle::SessionOutcome::Done,
            SessionStatus::Cancelled => lifecycle::SessionOutcome::Cancelled,
            _ => lifecycle::SessionOutcome::Failed {
                rate_limited: self.session_looks_rate_limited(&rec, &store.config().budget),
            },
        };
        let cfg = store.config().dispatcher.clone();
        let event = lifecycle::on_ended(&mut ticket, outcome, pr.clone(), &cfg);
        if let Err(e) = store.save(&mut ticket, &body, &extra) {
            error!("cannot save {key} after session.ended: {e}");
        }
        ledger::append(store.vault_root(), &event);

        if ticket.status == Status::InReview {
            if let Some(p) = &pr {
                self.bus.emit(
                    EVENT_TICKET_REVIEWED,
                    json!({ FIELD_KEY: key, FIELD_PR: p.url }),
                );
            }
        }
        emit_ticket_status(&self.bus, &key, ticket.status.as_str());

        self.learn_reserve(&store, &ticket, &rec);

        self.tick(TICK_REASON_SESSION_ENDED);
    }

    /// Whether a finished session looks like it died of a rate limit rather than a real failure.
    /// True on a rejected snapshot, a matching `rate_limit_patterns` text, or a non-zero exit at 5h saturation.
    fn session_looks_rate_limited(
        &self,
        rec: &crate::sessions::SessionRecord,
        budget_cfg: &aidp_tickets::config::BudgetConfig,
    ) -> bool {
        let snap = self.sessions.rate_limit();

        if snap.as_ref().map(|s| s.status.as_str()) == Some(RATE_LIMIT_REJECTED) {
            return true;
        }

        if let Some(text) = &rec.last_text {
            let lower = text.to_lowercase();
            if budget_cfg
                .rate_limit_patterns
                .iter()
                .any(|p| lower.contains(&p.to_lowercase()))
            {
                return true;
            }
        }

        if let Some(code) = rec.exit_code {
            if code != 0 {
                let utilization_5h = snap
                    .as_ref()
                    .and_then(|s| s.five_hour.as_ref())
                    .map(|w| w.utilization)
                    .unwrap_or(0.0);
                if utilization_5h >= budget_cfg.max_utilization_5h {
                    return true;
                }
            }
        }

        false
    }

    /// Feed `BudgetStats` from the before/after rate-limit snapshots of one dispatcher session.
    /// Best effort and guarded on `budget_cfg.learn`; failures are logged and never break `session.ended`.
    fn learn_reserve(
        &self,
        store: &TicketStore,
        ticket: &aidp_tickets::model::Ticket,
        rec: &crate::sessions::SessionRecord,
    ) {
        let budget_cfg = &store.config().budget;
        if !budget_cfg.learn {
            return;
        }
        let Some(template) = ticket.template.clone() else {
            return;
        };

        let started_at_ms = DateTime::parse_from_rfc3339(&rec.started_at)
            .map(|d| d.timestamp_millis())
            .unwrap_or(0);
        let now = now_ms();

        let history = self.sessions.rate_limit_history(0);

        let before = history
            .iter()
            .find(|s| s.observed_at <= started_at_ms)
            .map(to_rate_snapshot);

        let after = history
            .first()
            .filter(|s| {
                s.five_hour
                    .as_ref()
                    .is_some_and(|w| w.resets_at * 1000 > now)
            })
            .map(to_rate_snapshot);

        let (Some(before), Some(after)) = (before, after) else {
            return;
        };

        let concurrency = self
            .sessions
            .list(Some(SessionStatus::Running), None, 0)
            .iter()
            .filter(|r| dispatcher_ticket_key(&r.origin).is_some())
            .count() as f64
            + 1.0;

        let sample = budget::Sample {
            before,
            after,
            concurrency,
        };

        let mut stats = budget::BudgetStats::load(&self.vault_root);
        if !stats.record(&template, &sample, now) {
            return;
        }
        if let Err(e) = stats.save(&self.vault_root) {
            error!("cannot save budget-stats.json: {e}");
        }
    }

    fn on_session_question(&self, data: &Value) {
        let Some(ResolvedBusEvent {
            store,
            mut ticket,
            body,
            extra,
            key,
            rec,
        }) = self.resolve_bus_event(data)
        else {
            return;
        };
        let session_id = rec.id.as_str();

        let cfg = store.config().dispatcher.clone();
        if let Some(started_at) = ticket.autopilot.started_at {
            let hours = (SystemClock.now_utc() - started_at).num_hours();
            if hours >= i64::from(cfg.gate_timeout_hours) {
                let _ = self.sessions.cancel(session_id);
                let from = ticket.status;
                if let Some(event) = guard_and_build_event(
                    "cannot block after gate-timeout",
                    &ticket.key,
                    from,
                    Status::Blocked,
                    GATE_TIMEOUT_REASON.to_string(),
                    Some(session_id.to_string()),
                ) {
                    ticket.status = Status::Blocked;
                    ticket.autopilot.blocked_reason = Some(GATE_TIMEOUT_REASON.to_string());
                    if let Err(e) = store.save(&mut ticket, &body, &extra) {
                        error!("cannot save {key} after gate-timeout: {e}");
                    }
                    ledger::append(store.vault_root(), &event);
                    emit_ticket_status(&self.bus, &key, ticket.status.as_str());
                }
                return;
            }
        }

        let run_dir =
            crate::runs::resolve_run(&rec.cwd, rec.workflow_run_id.as_deref()).map(|r| r.run_dir);
        let stage = run_dir
            .as_deref()
            .and_then(crate::runs::pending_gate_stage)
            .or_else(|| parse_stage_from_question_text(data))
            .unwrap_or_else(|| UNKNOWN_STAGE.to_string());

        let event = lifecycle::on_gate(&mut ticket, &stage);
        if let Err(e) = store.save(&mut ticket, &body, &extra) {
            error!("cannot save {key} after gate: {e}");
        }
        ledger::append(store.vault_root(), &event);
        emit_ticket_status(&self.bus, &key, ticket.status.as_str());
    }

    fn on_session_answered(&self, data: &Value) {
        let Some(ResolvedBusEvent {
            store,
            mut ticket,
            body,
            extra,
            key,
            rec: _,
        }) = self.resolve_bus_event(data)
        else {
            return;
        };

        let event = lifecycle::on_gate_answered(&mut ticket);
        if let Err(e) = store.save(&mut ticket, &body, &extra) {
            error!("cannot save {key} after gate-answered: {e}");
        }
        ledger::append(store.vault_root(), &event);
        emit_ticket_status(&self.bus, &key, ticket.status.as_str());
    }
}

#[cfg(test)]
#[path = "../tests/unit/dispatcher_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../tests/unit/dispatcher_launch_tests.rs"]
mod launch_tests;
