//! HTTP API and SSE of the daemon, one thread per request; the `/api` prefix is optional.
//! A request body is bounded by [`MAX_BODY_BYTES`], checked after the access gate.

use crate::automations::AutomationEngine;
use crate::bus::Bus;
use crate::crypto::{ct_eq, hmac_sha256_hex};
use crate::models::attention_list_response::AttentionListResponse;
use crate::models::attention_read_request::AttentionReadRequest;
use crate::models::attention_read_response::AttentionReadResponse;
use crate::models::comment_state_request::CommentStateRequest;
use crate::models::constants::runtime_api::TOKEN_REQUIRED;
use crate::models::enums::static_lookup::StaticLookup;
use crate::models::enums::{ApiStatus, SessionStatus};
use crate::models::pr_detail_context::PrDetailContext;
use crate::models::pr_detail_query::PrDetailQuery;
use crate::models::pr_mark_request::PrMarkRequest;
use crate::models::squad_prs_query::SquadPrsQuery;
use crate::models::ticket_flow_request::TicketFlowRequest;
use crate::models::worktree_ensure_response::WorktreeEnsureResponse;
use crate::models::worktree_key_request::WorktreeKeyRequest;
use crate::models::worktree_remove_response::WorktreeRemoveResponse;
use crate::models::{AdoptRequest, ApiError};
use crate::observability::loggable_path;
use crate::paths::{skill_root, state_dir};
use crate::pr_detail_routes as prd;
use crate::presets::Presets;
use crate::projects::{Project, Projects};
use crate::session_error::SessionError;
use crate::sessions::{Origin, SessionManager, StartRequest};
use crate::time::{now_iso, now_ms};
use aidp_core::clock::{Clock, SystemClock};
use aidp_core::process_runner::OsProcessRunner;
use aidp_integrations::models::enums::comment_kind::CommentKind;
use aidp_integrations::{HasErrorKind, IntegrationError};
use aidp_tickets::ledger::{self, Event};
use aidp_tickets::model::{self, Actor, Autopilot, IssueType, Priority, Source, Status, Ticket};
use aidp_tickets::models::enums::adoption_reason::AdoptionReason;
use aidp_tickets::note;
use aidp_tickets::store::{ListFilter, TicketStore};
use aidp_tickets::{TicketError, Track};
use anyhow::anyhow;
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::io::Read;
use std::net::{IpAddr, SocketAddr, TcpListener};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{
    channel, sync_channel, Receiver, RecvTimeoutError, SyncSender, TrySendError,
};
use std::sync::Arc;
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};
use tracing::{error, info, info_span, warn};

const HTTP_OK: u16 = 200;
/// Body field of `PUT /api/projects/:id`: the test checkout (`null` or blank clears it).
const BODY_TEST_CHECKOUT: &str = "testCheckout";
const TEST_CHECKOUT_NOT_STRING: &str = "testCheckout must be a string or null";
/// Segment of `POST /api/projects/discover`.
const DISCOVER_SEGMENT: &str = "discover";
/// Body key listing the folders `discover` scans; the parents of the registered checkouts when absent.
const BODY_ROOTS: &str = "roots";
const WORKTREE_CAP_REASON: &str = "worktree_cap";
const TICKET_NOT_FOUND_MESSAGE: &str = "no such ticket";
const UNKNOWN_PROJECT_MESSAGE: &str = "no project registered for the ticket";
const HTTP_NO_CONTENT: u16 = 204;
const HTTP_BAD_REQUEST: u16 = 400;
const HTTP_INTERNAL_ERROR: u16 = 500;
const HTTP_UNAUTHORIZED: u16 = 401;
const HTTP_FORBIDDEN: u16 = 403;
const HTTP_NOT_FOUND: u16 = 404;
const HTTP_CONFLICT: u16 = 409;
const HTTP_PAYLOAD_TOO_LARGE: u16 = 413;
const HTTP_LOCKED: u16 = 423;
const HTTP_SERVICE_UNAVAILABLE: u16 = 503;
const DEFAULT_BODY_CAPACITY: usize = 8192;
const HEADER_CONTENT_TYPE: &str = "Content-Type";
const HEADER_CACHE_CONTROL: &str = "Cache-Control";
const HEADER_CONTENT_SECURITY_POLICY: &str = "Content-Security-Policy";
const HEADER_CONTENT_TYPE_OPTIONS: &str = "X-Content-Type-Options";
/// GitHub images do not change at a given URL: the browser keeps them a day, for this user only.
const CACHE_PRIVATE_DAY: &str = "private, max-age=86400";
const CSP_SANDBOX: &str = "default-src 'none'; style-src 'unsafe-inline'; sandbox";
const NOSNIFF: &str = "nosniff";
const QUERY_URL: &str = "url";
const ASSET_URL_MESSAGE: &str = "url is required";
const HEADER_ORIGIN: &str = "Origin";
const HEADER_VARY: &str = "Vary";
const HEADER_AUTHORIZATION: &str = "Authorization";
const HEADER_AIDP_TOKEN: &str = "X-Aidp-Token";
const HEADER_HUB_SIGNATURE: &str = "X-Hub-Signature-256";
const HEADER_GITHUB_EVENT: &str = "X-GitHub-Event";
const QUERY_REPO: &str = "repo";
const QUERY_NUMBER: &str = "number";
const QUERY_REFRESH: &str = "refresh";
const QUERY_REFRESH_ON: &str = "1";
const QUERY_UNREAD: &str = "unread";
const QUERY_UNREAD_ON: &str = "true";
const ATTENTION_READ_EVENT: &str = "pr.attention_read";
const ATTENTION_READ_MESSAGE: &str = "fingerprints or all=true is required";
const NO_PROJECT_MESSAGE_PREFIX: &str = "no project for service ";
const PR_PARAMS_MESSAGE: &str = "repo and a numeric number are required";
const HEADER_ALLOW_ORIGIN: &str = "Access-Control-Allow-Origin";
const HEADER_ALLOW_HEADERS: &str = "Access-Control-Allow-Headers";
const HEADER_ALLOW_METHODS: &str = "Access-Control-Allow-Methods";
const CORS_ALLOWED_HEADERS: &str = "Content-Type, Authorization, X-Aidp-Token";
const CONTENT_TYPE_JSON: &str = "application/json";
const CONTENT_TYPE_EVENT_STREAM: &str = "text/event-stream";
const CONTENT_TYPE_MARKDOWN: &str = "text/markdown; charset=utf-8";
const CONTENT_TYPE_TEXT: &str = "text/plain; charset=utf-8";
const CACHE_NO_CACHE: &str = "no-cache";
const BEARER_PREFIX: &str = "Bearer ";
const HMAC_PREFIX: &str = "sha256=";
const INVALID_HEADER_MESSAGE: &str = "valid header";
const SEGMENT_HOOKS: &str = "hooks";
const SEGMENT_EVENTS: &str = "events";
const ORIGIN_API: &str = "api";
const ORIGIN_UI: &str = "ui";
const ENGINE_PROMPT: &str = "prompt";
const DEFAULT_USAGE_GROUP: &str = "day";
const ENV_HOOK_TOKEN: &str = "AIDP_HOOK_TOKEN";
const ENV_UI_TOKEN: &str = "AIDP_UI_TOKEN";
const ENV_READ_ONLY: &str = "AIDP_READ_ONLY";
const ENV_MAX_BODY_BYTES: &str = "AIDP_MAX_BODY_BYTES";
/// Minimum `AIDP_UI_TOKEN` length to listen off-loopback.
const MIN_LAN_TOKEN_LEN: usize = 32;
const ENV_ALLOWED_ORIGINS: &str = "AIDP_ALLOWED_ORIGINS";
const QUERY_ES: &str = "es";
const QUERY_SINCE: &str = "since";
const QUERY_LIMIT: &str = "limit";
const QUERY_STATUS: &str = "status";
const QUERY_PROJECT_ID: &str = "projectId";
const QUERY_PROJECT: &str = "project";
const QUERY_TEXT: &str = "q";
const QUERY_FILE: &str = "file";
const QUERY_PR: &str = "pr";
const QUERY_FROM: &str = "from";
const QUERY_TO: &str = "to";
const QUERY_GROUP_BY: &str = "groupBy";
const QUERY_KEYS: &str = "keys";
const CACHE_INVALIDATOR_THREAD: &str = "cache-invalidator";
const MEMORY_EVENT_PREFIX: &str = "memory.";
const RUN_EVENTS: [&str; 3] = [
    crate::native_runs::EVENT_RUN_STARTED,
    crate::native_runs::EVENT_RUN_STAGE_CHANGED,
    crate::native_runs::EVENT_RUN_ENDED,
];
const QUERY_SERVICE: &str = "service";
const QUERY_SOURCE: &str = "source";
const SESSION_NOT_FOUND: &str = "session not found";
const TICKET_COST_NOT_FOUND: &str = "no session or note for this ticket";
const AUTOMATION_NOT_FOUND: &str = "automation not found";
const PROJECT_NOT_FOUND: &str = "project not found";
const HOME_NOT_FOUND: &str = "cannot locate the home directory";
const PRESET_NOT_FOUND: &str = "preset not found";
const QUEUE_ENTRY_NOT_FOUND: &str = "queue entry not found";
const ROUTE_NOT_FOUND: &str = "not found";
const UNAUTHORIZED_MESSAGE: &str = "unauthorized";
const FORBIDDEN_ORIGIN_MESSAGE: &str = "forbidden origin";
const READ_ONLY_MESSAGE: &str = "read-only mode: set AIDP_READ_ONLY=0 to allow writes";
const INVALID_JSON_MESSAGE: &str = "invalid JSON body";
const NO_VAULT_MESSAGE: &str = "no vault configured";
const SERVICE_REQUIRED_MESSAGE: &str = "service is required";
const SERVICE_FIELD: &str = "service";
const TEMPLATE_FIELD: &str = "template";
const TRACK_FIELD: &str = "track";
const FLOW_FIELD: &str = "flow";
const SUMMARY_REQUIRED_MESSAGE: &str = "summary is required";
const STATUS_REQUIRED_MESSAGE: &str = "status is required";
const PATH_REQUIRED_MESSAGE: &str = "path is required";
const DISPATCHER_PAUSED_MESSAGE: &str = "dispatcher paused";
const FLOWS_SEGMENT: &str = "flows";
const FLOWS_UNAVAILABLE_MESSAGE: &str = "flows unavailable: no vault";
const WEBHOOKS_DISABLED_MESSAGE: &str = "webhooks disabled: set AIDP_HOOK_TOKEN";
const NO_DIFF_MESSAGE: &str = "no diff for that file";
const NO_RUN_LINKED_MESSAGE: &str = "no vault run linked to this session";
const MANUAL_START_REASON: &str = "manual-start";
const PR_REQUIRED_MESSAGE: &str = "pr (url or number) is required";
const CANCELLED_BY_USER: &str = "cancelled-by-user";
/// Published on every manual transition, as the dispatcher does for its own.
const EVENT_TICKET_STATUS: &str = "ticket.status";
const DEFAULT_RESUME_PROMPT: &str = "Continue the pipeline from where it stopped.";
const REASON_SKIPPED: &str = "skipped";
const MODE_PAUSED: &str = "paused";
const TICKET_TODO: &str = "todo";
const TICKET_QUEUED: &str = "queued";
const TICKET_IN_PROGRESS: &str = "in-progress";
const TICKET_IN_PROGRESS_SNAKE: &str = "in_progress";
const TICKET_IN_PROGRESS_FLAT: &str = "inprogress";
const TICKET_IN_REVIEW: &str = "in-review";
const TICKET_IN_REVIEW_SNAKE: &str = "in_review";
const TICKET_IN_REVIEW_FLAT: &str = "inreview";
const TICKET_DONE: &str = "done";
const TICKET_BLOCKED: &str = "blocked";
const TICKET_CANCELLED: &str = "cancelled";
const TICKET_CANCELED_US: &str = "canceled";
const SOURCE_GITHUB: &str = "github";
const SOURCE_LOCAL: &str = "local";
const READ_ONLY_TRUE_VALUES: &[&str] = &["1", "true", "yes", "on"];

pub struct App {
    pub bus: Arc<Bus>,
    pub projects: Arc<Projects>,
    pub sessions: Arc<SessionManager>,
    pub automations: Arc<AutomationEngine>,
    pub presets: Arc<Presets>,
    pub pr_status: Arc<crate::prs::PrStatusCache>,
    pub squad_prs: crate::squad_prs::SquadPrsCache,
    pub pr_marks: crate::pr_marks::PrMarks,
    pub pr_attention: crate::pr_attention::PrAttention,
    pub pr_comment_state: crate::pr_comment_state::PrCommentState,
    pub pr_drafts: crate::pr_drafts::PrDrafts,
    pub pr_viewed: crate::pr_viewed::PrViewed,
    pub github_rate_limit: crate::github_rate_limit::RateLimitCache,
    pub dispatcher: Arc<crate::dispatcher::Dispatcher>,
    pub started_at: String,
    pub started_ms: i64,
    pub hook_token: String,
    pub state_dir: PathBuf,
    pub vault_root: PathBuf,
    pub skill_root: Option<PathBuf>,
    pub ui_token: String,
    pub require_loopback_token: bool,
    pub read_only: bool,
    pub max_body_bytes: usize,
    /// Exact `Origin` values allowed to send mutating requests and to receive CORS headers
    /// (`AIDP_ALLOWED_ORIGINS`, comma-separated), plus the daemon's own loopback origins for
    /// its listening port, added at startup. Any other explicit Origin is refused.
    pub allowed_origins: Vec<String>,
    /// Single-use tokens minted by `POST /api/events/token` for `EventSource`, which cannot
    /// send headers. Consumed on first use, valid [`SSE_TOKEN_TTL`].
    pub sse_tokens: Mutex<HashMap<String, Instant>>,
    /// Request threads currently running, SSE connections included.
    pub inflight_requests: Arc<AtomicUsize>,
    /// Ceiling on [`App::inflight_requests`]; beyond it a request gets `503`.
    pub max_inflight_requests: usize,
    /// The ticket flow engine; `None` when no vault is configured.
    pub flows: Option<Arc<crate::flows::FlowEngine>>,
    /// Executor of native runs served under `/api/runs`.
    pub native_runs: Arc<crate::native_runs::NativeRunManager>,
    /// Chooses between a native run and a legacy session for every template launch.
    pub launcher: Arc<crate::launcher::RunLauncher>,
    /// Shared memory store, background re-index and run-end ingestion.
    pub memory: Arc<crate::memory_ingest::MemoryHub>,
    /// Web UI build answering the non-API reads (`--ui-dir`); empty serves none.
    pub static_ui: crate::static_ui::StaticUi,
    /// The pipeline doctor; `None` without a flow engine.
    pub doctor: Option<Arc<crate::doctor::Doctor>>,
    /// Stale-while-revalidate caches of the worktree and memory listings.
    pub route_caches: crate::route_caches::RouteCaches,
}

/// The JSON answer of a launch: a native run acknowledgement, or the session launch unchanged.
fn launched_reply(launched: crate::models::launched_run::LaunchedRun) -> Reply {
    use crate::models::enums::engine::Engine;
    use crate::models::launched_run::LaunchedRun;
    use crate::models::native_launch_accepted::NativeLaunchAccepted;
    match launched {
        LaunchedRun::Session(launch) => Reply::Json(HTTP_OK, launch.to_json()),
        LaunchedRun::Native { run_id } => {
            let accepted = NativeLaunchAccepted {
                ok: true,
                run_id,
                mode: Engine::Native,
            };
            Reply::Json(HTTP_OK, serde_json::to_value(accepted).unwrap_or_default())
        }
    }
}

/// Default ceiling on concurrently running request threads (override:
/// `AIDP_MAX_INFLIGHT_REQUESTS`, see [`parse_max_inflight_requests`]).
pub const MAX_INFLIGHT_REQUESTS: usize = 256;

/// Error message of the `503` answered when every request slot is taken.
pub const TOO_MANY_REQUESTS_MESSAGE: &str = "too many requests in flight";

/// HTTP status answered when every request slot is taken.
const SERVICE_UNAVAILABLE: u16 = 503;

/// Environment variable overriding [`MAX_INFLIGHT_REQUESTS`].
const MAX_INFLIGHT_REQUESTS_ENV: &str = "AIDP_MAX_INFLIGHT_REQUESTS";

/// Frames an SSE stream buffers before its forwarder waits; exceeds the
/// connect frame plus the 100-event replay, so a replay never waits.
pub const SSE_CHANNEL_CAPACITY: usize = 256;

/// Pause between two attempts to push a frame into a full SSE channel.
const SSE_RETRY_INTERVAL: Duration = Duration::from_millis(50);

/// How long an SSE channel may stay full before the client counts as stalled.
const SSE_STALL_GRACE: Duration = Duration::from_secs(5);

/// Bus event type emitted once when an SSE stream is closed for stalling.
pub const SSE_OVERFLOW_EVENT: &str = "overflow";

/// `stream` value of an overflow event raised by `GET /api/events`.
const SSE_STREAM_EVENTS: &str = "events";

/// `stream` value of an overflow event raised by a session stream.
const SSE_STREAM_SESSION: &str = "session";

/// How often `serve()` checks for a stop signal or a finished accept thread.
const SIGNAL_POLL_INTERVAL: Duration = Duration::from_secs(1);

/// Pure half of the `AIDP_MAX_INFLIGHT_REQUESTS` override: an absent, empty,
/// non-numeric, zero or negative value falls back to [`MAX_INFLIGHT_REQUESTS`].
pub fn parse_max_inflight_requests(raw: Option<&str>) -> usize {
    raw.and_then(parse_positive_byte_count)
        .unwrap_or(MAX_INFLIGHT_REQUESTS)
}

/// [`parse_max_inflight_requests`] over the process environment, read once in `serve_on()`.
fn max_inflight_requests_from_env() -> usize {
    parse_max_inflight_requests(std::env::var(MAX_INFLIGHT_REQUESTS_ENV).ok().as_deref())
}

/// One taken request slot; dropping it, panic included, releases the slot.
struct InflightSlot {
    counter: Arc<AtomicUsize>,
}

impl InflightSlot {
    fn acquire(counter: &Arc<AtomicUsize>) -> Self {
        counter.fetch_add(1, Ordering::SeqCst);
        InflightSlot {
            counter: Arc::clone(counter),
        }
    }
}

impl Drop for InflightSlot {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Answers `503` on the accept thread, without spawning, when every slot is taken.
fn refuse_saturated(request: Request) {
    let logged_path = loggable_path(request.url());
    warn!(method = %request.method(), path = %logged_path, status = SERVICE_UNAVAILABLE, "request refused: too many in flight");
    let body = json!({ "error": TOO_MANY_REQUESTS_MESSAGE }).to_string();
    let mut response =
        Response::from_string(body).with_status_code(StatusCode(SERVICE_UNAVAILABLE));
    response.add_header(hdr(HEADER_CONTENT_TYPE, CONTENT_TYPE_JSON));
    let _ = request.respond(response);
}

/// Lifetime of an ephemeral SSE token.
const SSE_TOKEN_TTL: Duration = Duration::from_secs(30);

/// Largest request body read, in bytes; above it the request is answered `413`.
/// Override with `AIDP_MAX_BODY_BYTES`, see [`parse_max_body_bytes`].
pub const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;

fn parse_positive_byte_count(raw: &str) -> Option<usize> {
    raw.trim().parse::<usize>().ok().filter(|&n| n > 0)
}

/// Pure half of the `AIDP_MAX_BODY_BYTES` override: an invalid or zero value
/// falls back to [`MAX_BODY_BYTES`] rather than disabling the bound.
pub fn parse_max_body_bytes(raw: Option<&str>) -> usize {
    raw.and_then(parse_positive_byte_count)
        .unwrap_or(MAX_BODY_BYTES)
}

/// [`parse_max_body_bytes`] over the process environment, read in `serve_on()`.
pub fn max_body_bytes_from_env() -> usize {
    parse_max_body_bytes(std::env::var(ENV_MAX_BODY_BYTES).ok().as_deref())
}

/// Configuration for starting an HTTP gateway daemon on a pre-bound listener.
pub struct HttpConfig {
    pub state_dir: PathBuf,
    pub vault_root: PathBuf,
    pub hook_token: String,
    pub ui_token: String,
    pub read_only: bool,
    pub allowed_origins: Vec<String>,
}

/// Handle to a running gateway HTTP server, holding the bound port and references to
/// cleanly shut down the daemon.
pub struct ServeHandle {
    app: Arc<App>,
    server: Arc<Server>,
    port: u16,
    sessions: Arc<SessionManager>,
    shutdown: Arc<crate::shutdown::Shutdown>,
    accept_thread: Option<std::thread::JoinHandle<()>>,
    flows_background: Option<crate::flows::recovery::Background>,
    doctor_thread: Option<std::thread::JoinHandle<()>>,
}

impl ServeHandle {
    /// Shared runtime services; direct Rust callers enforce their own access policy.
    pub fn app(&self) -> &Arc<App> {
        &self.app
    }

    /// The port the listener is actually bound to (never 0), from `Server::server_addr()`.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Stops admission, owned work and background services before returning.
    pub fn stop(mut self) {
        if let Err(error) = self.try_stop() {
            tracing::error!(%error, "gateway shutdown could not confirm every owned process stopped");
        }
    }

    /// Retains the handle on failure so embedded hosts keep their ownership lock.
    pub fn try_stop(&mut self) -> anyhow::Result<()> {
        self.shutdown.set();
        self.server.unblock();
        if let Some(t) = self.accept_thread.take() {
            let _ = t.join();
        }
        if let Some(background) = self.flows_background.take() {
            background.join();
        }
        if let Some(doctor) = self.doctor_thread.take() {
            let _ = doctor.join();
        }
        if let Some(flows) = &self.app.flows {
            flows.shutdown()?;
        }
        self.app.native_runs.try_shutdown()?;
        self.sessions.try_shutdown()?;
        Ok(())
    }

    /// True once the accept thread has exited (or was already joined).
    pub fn is_finished(&self) -> bool {
        self.accept_thread
            .as_ref()
            .is_none_or(|accept_thread| accept_thread.is_finished())
    }
}

enum Reply {
    Json(u16, Value),
    Empty(u16),
    Stream(Box<dyn Read + Send + 'static>),
    Text(u16, String, &'static str),
    /// An image read from GitHub, served sandboxed so an SVG opened alone runs no script.
    Asset(aidp_integrations::models::github_asset::GithubAsset),
    /// A file of the web UI build (`--ui-dir`).
    Static(crate::models::static_file::StaticFile),
}

fn err(code: u16, msg: impl ToString) -> Reply {
    let status = ApiStatus::from_status(code);
    Reply::Json(
        code,
        json!({ "error": msg.to_string(), "code": status.code() }),
    )
}

fn ok() -> Reply {
    Reply::Json(HTTP_OK, json!({ "ok": true }))
}

/// Answers a pause change with the effective state, or a 500 carrying the error message.
fn dispatcher_pause_reply(result: anyhow::Result<bool>) -> Reply {
    match result {
        Ok(paused) => match serde_json::to_value(crate::models::DispatcherPauseResponse { paused })
        {
            Ok(value) => Reply::Json(HTTP_OK, value),
            Err(error) => err(HTTP_INTERNAL_ERROR, error),
        },
        Err(error) => err(HTTP_INTERNAL_ERROR, error),
    }
}

/// Maps a PR route result onto its HTTP reply, keeping the machine-read `reason` of a 404.
fn pr_reply(result: Result<Value, crate::prs::PrRouteError>) -> Reply {
    match result {
        Ok(value) => Reply::Json(HTTP_OK, value),
        Err(error) => Reply::Json(error.status(), error.body()),
    }
}

/// Runs a PR page handler with the shared caches, stores and the real `gh` runner.
fn pr_detail_call<T>(
    app: &App,
    input: Result<T, String>,
    handler: impl FnOnce(&PrDetailContext<'_>, T) -> Result<Value, crate::prs::PrRouteError>,
) -> Reply {
    pr_detail_call_in(app, None, input, handler)
}

/// Like `pr_detail_call`, with the checkout the handler may read.
fn pr_detail_call_in<T>(
    app: &App,
    workspace_root: Option<&std::path::Path>,
    input: Result<T, String>,
    handler: impl FnOnce(&PrDetailContext<'_>, T) -> Result<Value, crate::prs::PrRouteError>,
) -> Reply {
    let input = match input {
        Ok(input) => input,
        Err(message) => return err(HTTP_BAD_REQUEST, message),
    };
    let runner = aidp_core::process_runner::OsProcessRunner;
    let ctx = PrDetailContext {
        cache: &app.pr_status,
        bus: &app.bus,
        squad: &app.squad_prs,
        drafts: &app.pr_drafts,
        viewed: &app.pr_viewed,
        runner: &runner,
        workspace_root,
    };
    pr_reply(handler(&ctx, input))
}

/// Parses a request body, mapping a failure onto the message of a 400.
fn pr_body<R: serde::de::DeserializeOwned>(body: Value) -> Result<R, String> {
    serde_json::from_value(body).map_err(|error| format!("invalid body: {error}"))
}

/// Reads the PR page query parameters.
fn pr_query(q: &std::collections::HashMap<String, String>) -> Result<PrDetailQuery, String> {
    PrDetailQuery::from_lookup(|key| q.get(key).cloned().filter(|value| !value.is_empty()))
}

/// Parses a numeric comment id from the route.
fn pr_comment_id(raw: &str) -> Result<u64, String> {
    prd::parse_comment_id(raw).map_err(|error| error.error.message)
}

/// Parses the request body as `R`, runs the PR action `handler` and maps its result onto a reply.
fn pr_action<R: serde::de::DeserializeOwned>(
    app: &App,
    body: Value,
    handler: fn(
        &crate::pr_actions::PrActionContext<'_>,
        R,
    ) -> Result<Value, crate::prs::PrRouteError>,
) -> Reply {
    let request: R = match serde_json::from_value(body) {
        Ok(request) => request,
        Err(error) => return err(HTTP_BAD_REQUEST, format!("invalid body: {error}")),
    };
    let ctx = crate::pr_actions::PrActionContext {
        cache: &app.pr_status,
        bus: &app.bus,
        vault_root: &app.vault_root,
    };
    pr_reply(handler(&ctx, request))
}

/// The Claude Code transcripts recorded for a project path, newest first.
fn list_claude_sessions(project_path: &str) -> Reply {
    match aidp_core::vault::home_dir() {
        Some(home) => Reply::Json(
            HTTP_OK,
            json!(crate::claude_sessions::list(&home, project_path)),
        ),
        None => api_err(ApiError {
            status: ApiStatus::Config,
            message: HOME_NOT_FOUND.to_string(),
        }),
    }
}

fn api_err(e: impl Into<ApiError>) -> Reply {
    let api_error: ApiError = e.into();
    Reply::Json(
        api_error.status.status(),
        json!({ "error": api_error.message, "code": api_error.status.code() }),
    )
}

/// Converts the first typed error of the chain to an [`ApiStatus`] through its
/// `ErrorKind`, so a new typed error needs only a `HasErrorKind` impl.
impl From<&anyhow::Error> for ApiError {
    fn from(e: &anyhow::Error) -> Self {
        if let Some(session_err) = e.chain().find_map(|c| c.downcast_ref::<SessionError>()) {
            return ApiError {
                status: session_err.kind().into(),
                message: session_err.to_string(),
            };
        }
        if let Some(refusal) = e
            .chain()
            .find_map(|c| c.downcast_ref::<crate::models::ApiRefusal>())
        {
            return ApiError {
                status: refusal.status,
                message: refusal.message.clone(),
            };
        }
        if let Some(ticket_err) = e.chain().find_map(|c| c.downcast_ref::<TicketError>()) {
            return ApiError {
                status: ticket_err.kind().into(),
                message: ticket_err.to_string(),
            };
        }
        if let Some(integration_err) = e.chain().find_map(|c| c.downcast_ref::<IntegrationError>())
        {
            return ApiError {
                status: integration_err.kind().into(),
                message: integration_err.to_string(),
            };
        }
        if e.chain()
            .any(|c| c.downcast_ref::<std::io::Error>().is_some())
        {
            return ApiError {
                status: ApiStatus::Io,
                message: e.to_string(),
            };
        }
        ApiError {
            status: ApiStatus::BadRequest,
            message: e.to_string(),
        }
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        (&e).into()
    }
}

/// Keeps only the gates in `SELF_APPROVABLE_GATES_FROM_HTTP`; applied to the public
/// `POST /api/sessions` body only. An empty result maps to `None`.
fn sanitize_auto_approve_gates(gates: Option<Vec<String>>) -> Option<Vec<String>> {
    let filtered: Vec<String> = gates?
        .into_iter()
        .filter(|g| crate::sessions::SELF_APPROVABLE_GATES_FROM_HTTP.contains(&g.trim()))
        .collect();
    if filtered.is_empty() {
        None
    } else {
        Some(filtered)
    }
}

pub fn serve_on(listener: TcpListener, config: HttpConfig) -> anyhow::Result<ServeHandle> {
    serve_on_with_auth(
        listener,
        config,
        false,
        crate::models::runtime_environment::RuntimeEnvironment {
            skill_root: skill_root(),
            ..Default::default()
        },
    )
}

/// [`serve_on`] with explicit runtime configuration, the UI directory included.
pub fn serve_on_with_environment(
    listener: TcpListener,
    config: HttpConfig,
    environment: crate::models::runtime_environment::RuntimeEnvironment,
) -> anyhow::Result<ServeHandle> {
    serve_on_with_auth(listener, config, false, environment)
}

/// Starts an embedded server requiring the UI token from loopback clients as well.
pub fn serve_on_authenticated(
    listener: TcpListener,
    config: HttpConfig,
) -> anyhow::Result<ServeHandle> {
    anyhow::ensure!(!config.ui_token.is_empty(), TOKEN_REQUIRED);
    serve_on_embedded(
        listener,
        config,
        crate::models::runtime_environment::RuntimeEnvironment {
            skill_root: skill_root(),
            ..Default::default()
        },
    )
}

/// Starts an authenticated runtime with explicit skill and child process configuration.
pub fn serve_on_embedded(
    listener: TcpListener,
    config: HttpConfig,
    environment: crate::models::runtime_environment::RuntimeEnvironment,
) -> anyhow::Result<ServeHandle> {
    anyhow::ensure!(!config.ui_token.is_empty(), TOKEN_REQUIRED);
    serve_on_with_auth(listener, config, true, environment)
}

fn serve_on_with_auth(
    listener: TcpListener,
    mut config: HttpConfig,
    require_loopback_token: bool,
    environment: crate::models::runtime_environment::RuntimeEnvironment,
) -> anyhow::Result<ServeHandle> {
    let static_ui = crate::static_ui::StaticUi::open(environment.ui_dir.as_deref())?;
    crate::store::ensure_dir(&config.state_dir);
    let shutdown = Arc::new(crate::shutdown::Shutdown::new());

    let port = listener.local_addr()?.port();
    let bound_ip = listener.local_addr()?.ip();
    let lan_ips = if bound_ip.is_unspecified() {
        detect_lan_ips()
    } else {
        Vec::new()
    };
    add_own_origins(&mut config.allowed_origins, port, bound_ip, &lan_ips);
    let gateway_url = format!(
        "{}:{port}",
        if listener.local_addr()?.is_ipv6() {
            crate::models::constants::flow_execution::LOOPBACK_HTTP_V6
        } else {
            crate::models::constants::flow_execution::LOOPBACK_HTTP
        }
    );

    let bus = Arc::new(Bus::new());
    let projects = Arc::new(Projects::load(&config.state_dir));
    let mut child_environment = environment.child_environment;
    if let Some(root) = &environment.skill_root {
        child_environment.insert(
            crate::models::constants::flow_execution::SKILL_ROOT_ENV.to_string(),
            root.to_string_lossy().to_string(),
        );
    }
    child_environment.extend(runtime_environment(&config, &gateway_url));
    let sessions = SessionManager::load(&config.state_dir, bus.clone(), projects.clone())
        .with_runtime_environment(child_environment);
    for (configuration_id, configuration) in environment.initial_mcp_configurations {
        sessions
            .mcp_configurations()
            .upsert(configuration_id, configuration)?;
    }
    let native_runs = build_native_runs(&config, &sessions, &projects, &bus);
    let launcher = Arc::new(crate::launcher::RunLauncher::new(
        sessions.clone(),
        native_runs.clone(),
    ));
    let automations = AutomationEngine::load(
        &config.state_dir,
        sessions.clone(),
        projects.clone(),
        bus.clone(),
    );
    automations.set_launcher(launcher.clone());
    automations.start_threads(shutdown.clone());
    crate::runs::start_watcher(sessions.clone(), bus.clone(), shutdown.clone());
    let presets = Arc::new(Presets::load(&config.state_dir, bus.clone()));
    let dispatcher = crate::dispatcher::Dispatcher::new_with_launcher(
        config.vault_root.clone(),
        sessions.clone(),
        launcher.clone(),
        projects.clone(),
        bus.clone(),
    );
    dispatcher.spawn(shutdown.clone());
    let pr_status = Arc::new(crate::prs::PrStatusCache::new(Box::new(
        aidp_core::clock::SystemClock,
    )));
    let (flows, flows_background) = start_flow_engine(
        &config, &sessions, &projects, &bus, &shutdown, &pr_status, &launcher,
    )
    .unzip();
    if let Some(engine) = flows.as_ref() {
        dispatcher.attach_flows(engine.clone());
    }

    native_runs.resume_all();

    let squad_prs = crate::squad_prs::SquadPrsCache::persisted(
        Box::new(aidp_core::clock::SystemClock),
        Box::new(crate::squad_prs::GhSquadFetcher::new(
            projects.clone(),
            &config.state_dir,
        )),
        config
            .state_dir
            .join(crate::squad_prs::SQUAD_REPOS_CACHE_DIR)
            .join(crate::squad_prs::SQUAD_SNAPSHOT_FILE),
    );
    let pr_marks = crate::pr_marks::PrMarks::new(&config.state_dir);
    let pr_attention = crate::pr_attention::PrAttention::new(&config.state_dir);
    let pr_comment_state = crate::pr_comment_state::PrCommentState::new(&config.state_dir);
    let pr_drafts = crate::pr_drafts::PrDrafts::new(&config.state_dir);
    let pr_viewed = crate::pr_viewed::PrViewed::new(&config.state_dir);
    let doctor = flows
        .as_ref()
        .map(|engine| build_doctor(&config, engine, &native_runs, &projects));
    let doctor_thread = doctor.as_ref().map(|doctor| doctor.spawn(shutdown.clone()));
    let memory =
        crate::memory_ingest::MemoryHub::new(&config.state_dir, &config.vault_root, bus.clone());
    memory.spawn_listener(native_runs.clone(), projects.clone());
    let app = Arc::new(App {
        bus,
        projects,
        sessions: sessions.clone(),
        automations,
        presets,
        pr_status,
        squad_prs,
        pr_marks,
        pr_attention,
        pr_comment_state,
        pr_drafts,
        pr_viewed,
        github_rate_limit: crate::github_rate_limit::RateLimitCache::new(
            Box::new(aidp_core::clock::SystemClock),
            Box::new(crate::github_rate_limit::GhRateLimitReader),
        ),
        dispatcher,
        started_at: now_iso(),
        started_ms: now_ms(),
        hook_token: config.hook_token,
        state_dir: config.state_dir,
        vault_root: config.vault_root,
        skill_root: environment.skill_root,
        ui_token: config.ui_token,
        require_loopback_token,
        read_only: config.read_only,
        max_body_bytes: max_body_bytes_from_env(),
        allowed_origins: config.allowed_origins,
        sse_tokens: Mutex::new(HashMap::new()),
        inflight_requests: Arc::new(AtomicUsize::new(0)),
        max_inflight_requests: max_inflight_requests_from_env(),
        flows,
        native_runs,
        launcher,
        memory,
        static_ui,
        doctor,
        route_caches: crate::route_caches::RouteCaches::default(),
    });
    crate::vault_index::warm(&app.vault_root, &app.state_dir);
    spawn_cache_invalidator(&app);

    let server =
        Arc::new(Server::from_listener(listener, None).map_err(|e| anyhow!("from_listener: {e}"))?);

    let services = Arc::clone(&app);
    let server_clone = Arc::clone(&server);
    let accept_thread = std::thread::spawn(move || {
        for request in server_clone.incoming_requests() {
            if app.inflight_requests.load(Ordering::SeqCst) >= app.max_inflight_requests {
                refuse_saturated(request);
                continue;
            }
            let slot = InflightSlot::acquire(&app.inflight_requests);
            let app = app.clone();
            thread::spawn(move || {
                let _slot = slot;
                handle(app, request);
            });
        }
    });

    Ok(ServeHandle {
        app: services,
        server,
        port,
        sessions,
        shutdown,
        accept_thread: Some(accept_thread),
        flows_background,
        doctor_thread,
    })
}

/// Records attention events from the squad cache.
fn attention_pass(app: &App) {
    crate::squad_prs::refresh_attention(&app.squad_prs, &app.pr_marks, &app.pr_attention, &app.bus);
}

/// Refreshes a stale squad inbox on its own thread, then tells the viewers it changed.
fn spawn_inbox_refresh(app: &Arc<App>) {
    let app = Arc::clone(app);
    thread::spawn(move || {
        if app.squad_prs.refresh_once() {
            app.squad_prs.notify_changed(&app.bus);
        }
    });
}

/// Drops the cached reads a bus event outdates: memory listings on a memory event, and the run
/// index of the vault on a native run event.
fn spawn_cache_invalidator(app: &Arc<App>) {
    let events = app.bus.subscribe();
    let app = Arc::clone(app);
    let spawned = thread::Builder::new()
        .name(CACHE_INVALIDATOR_THREAD.to_string())
        .spawn(move || {
            while let Ok(event) = events.recv() {
                let kind = event["type"].as_str().unwrap_or_default();
                if kind.starts_with(MEMORY_EVENT_PREFIX) {
                    app.route_caches.memory.clear();
                } else if RUN_EVENTS.contains(&kind) {
                    crate::vault_index::invalidate(&app.vault_root);
                }
            }
        });
    if let Err(error) = spawned {
        tracing::warn!(%error, "cache invalidator not started");
    }
}

/// Runs an attention pass on its own thread; the cache fetch may take seconds.
fn spawn_attention_pass(app: &Arc<App>) {
    let app = Arc::clone(app);
    thread::spawn(move || attention_pass(&app));
}

fn runtime_environment(config: &HttpConfig, gateway_url: &str) -> BTreeMap<String, String> {
    use crate::models::constants::flow_execution::{
        GATEWAY_DIR_ENV, GATEWAY_URL_ENV, UI_TOKEN_ENV, VAULT_ENV,
    };
    BTreeMap::from([
        (GATEWAY_URL_ENV.to_string(), gateway_url.to_string()),
        (
            GATEWAY_DIR_ENV.to_string(),
            config.state_dir.to_string_lossy().to_string(),
        ),
        (UI_TOKEN_ENV.to_string(), config.ui_token.clone()),
        (
            VAULT_ENV.to_string(),
            if config.vault_root.as_os_str().is_empty() {
                crate::models::constants::flow_execution::VAULT_DISABLED.to_string()
            } else {
                config.vault_root.to_string_lossy().to_string()
            },
        ),
    ])
}

/// Builds the native run executor over the daemon's own session manager.
fn build_native_runs(
    config: &HttpConfig,
    sessions: &Arc<SessionManager>,
    projects: &Arc<Projects>,
    bus: &Arc<Bus>,
) -> Arc<crate::native_runs::NativeRunManager> {
    let clock: Arc<dyn aidp_core::clock::Clock> = Arc::new(aidp_core::clock::SystemClock);
    let budget = Arc::new(crate::flows::registry::RateLimitGuard::new(
        sessions.clone(),
        config.vault_root.clone(),
        clock,
    ));
    crate::native_runs::NativeRunManager::new(
        crate::native_runs::NativeDeps::new(
            sessions.clone(),
            projects.clone(),
            bus.clone(),
            budget,
            Arc::new(crate::models::LocalLock::default()),
            &config.state_dir,
            sessions.configured_skill_root(),
        )
        .with_default_vault(config.vault_root.clone()),
    )
}

/// Builds the flow engine and starts its resident threads; `None` without a vault.
fn start_flow_engine(
    config: &HttpConfig,
    sessions: &Arc<SessionManager>,
    projects: &Arc<Projects>,
    bus: &Arc<Bus>,
    shutdown: &Arc<crate::shutdown::Shutdown>,
    pr_status: &Arc<crate::prs::PrStatusCache>,
    launcher: &Arc<crate::launcher::RunLauncher>,
) -> Option<(
    Arc<crate::flows::FlowEngine>,
    crate::flows::recovery::Background,
)> {
    use crate::flows::{executor::Ports, registry, FlowEngine, FlowEngineConfig};
    if config.vault_root.as_os_str().is_empty() {
        return None;
    }
    let clock: Arc<dyn aidp_core::clock::Clock> = Arc::new(aidp_core::clock::SystemClock);
    let ports = Ports {
        sessions: Arc::new(
            registry::ManagerSessions::new(sessions.clone(), projects.clone(), launcher.clone())
                .with_vault_root(config.vault_root.clone()),
        ),
        budget: Arc::new(registry::RateLimitGuard::new(
            sessions.clone(),
            config.vault_root.clone(),
            clock.clone(),
        )),
        lock: Arc::new(crate::flows::lock::SessionLock::new(sessions.clone())),
        bus: bus.clone(),
        clock,
        ids: Arc::new(aidp_core::id::SystemIdGen),
        runner: Arc::new(crate::flows::process_runner::FlowProcessRunner::new(
            Arc::new(aidp_core::process_runner::OsProcessRunner),
            sessions.runtime_environment(),
            sessions
                .runtime_environment()
                .get(crate::models::constants::flow_execution::AIDP_BIN_ENV)
                .cloned()
                .or_else(crate::flows::process_runner::aidp_program),
        )),
        lock_wait: crate::flows::FLOW_LOCK_WAIT,
    };
    let flow_dirs = sessions
        .configured_skill_root()
        .map(|root| vec![root.join(crate::flows::FLOW_TEMPLATES_DIR)])
        .unwrap_or_default();
    let engine = FlowEngine::new(
        config.vault_root.clone(),
        config.state_dir.clone(),
        registry::Registry::with_pr_status(
            pr_status.clone(),
            ticket_locator(
                config.vault_root.clone(),
                config.state_dir.clone(),
                projects.clone(),
            ),
        ),
        ports,
        FlowEngineConfig {
            flow_dirs,
            ..Default::default()
        },
    );
    crate::flows::recovery::resume_all(&engine);
    let background = crate::flows::recovery::spawn_background(
        &engine,
        shutdown.clone(),
        crate::flows::recovery::TICK_INTERVAL,
    );
    Some((engine, background))
}

/// Builds the pipeline doctor over the flow engine and the native runs. Its thread starts
/// later, outside the start path, and its first pass waits for the daemon to be up.
fn build_doctor(
    config: &HttpConfig,
    engine: &Arc<crate::flows::FlowEngine>,
    native_runs: &Arc<crate::native_runs::NativeRunManager>,
    projects: &Arc<Projects>,
) -> Arc<crate::doctor::Doctor> {
    use crate::models::constants::doctor::{FIRST_TICK_DELAY, TICK_INTERVAL};
    let vault = config.vault_root.clone();
    let unity_projects = projects.clone();
    let unity_engine = Arc::downgrade(engine);
    crate::doctor::Doctor::new(crate::doctor::DoctorDeps {
        engine: engine.clone(),
        runs: Arc::new(crate::doctor::ports::NativeDoctorRuns(native_runs.clone())),
        settings: Arc::new(move || aidp_tickets::config::load(&vault).doctor),
        state_dir: config.state_dir.clone(),
        unity: Some(Arc::new(move |project_id: &str| {
            let project = unity_projects.get(project_id)?;
            let engine = unity_engine.upgrade();
            Some(crate::unity_routes::project_status(
                &project,
                engine.as_ref(),
            ))
        })),
        unity_modals: Arc::new(crate::doctor::ports::NoUnityModals),
        harness_repos: harness_repos(skill_root()),
        first_delay: FIRST_TICK_DELAY,
        interval: TICK_INTERVAL,
    })
}

/// Local clones of the harness repositories: the skill's root, and the daemon's sources named by
/// `AIDP_SOURCE_ROOT`. A repository without a clone gets no pull request from the doctor.
fn harness_repos(skill_root: Option<PathBuf>) -> crate::doctor::ports::HarnessRepos {
    use crate::models::constants::doctor::HARNESS_REPOS;
    let source = std::env::var_os("AIDP_SOURCE_ROOT").map(PathBuf::from);
    Arc::new(move |repo: &str| {
        if repo == HARNESS_REPOS[0] {
            source.clone()
        } else if repo == HARNESS_REPOS[1] {
            skill_root.clone()
        } else {
            None
        }
    })
}

/// Resolves a ticket key to its ticket and the directory of its registered project.
fn ticket_locator(
    vault_root: PathBuf,
    state_dir: PathBuf,
    projects: Arc<Projects>,
) -> crate::models::TicketLocator {
    let flows = crate::flows::store::FlowStore::new(vault_root.clone(), state_dir);
    Arc::new(move |key| {
        let local = TicketStore::open(&vault_root)
            .ok()
            .and_then(|store| store.get(key).ok())
            .map(|(ticket, _, _)| ticket);
        let state = flows
            .index_get(key)
            .and_then(|entry| flows.load_state(&entry.repo, key));
        let ticket = local.or_else(|| {
            state
                .as_ref()
                .and_then(crate::flows::ticket_snapshot::ticket)
        })?;
        let project_name = state
            .as_ref()
            .and_then(|state| crate::flows::ticket_snapshot::project_name(&state.ticket))
            .unwrap_or(&ticket.service);
        let project = projects.get(project_name)?;
        Some((ticket, PathBuf::from(project.path)))
    })
}

/// Builds the listen address from a parsed IP, so an IPv6 host needs no bracket handling.
pub(crate) fn socket_addr(bind: IpAddr, port: u16) -> SocketAddr {
    SocketAddr::new(bind, port)
}

pub fn serve(
    bind: IpAddr,
    port: u16,
    state_dir_opt: Option<PathBuf>,
    ui_dir: Option<PathBuf>,
) -> anyhow::Result<()> {
    let dir = match state_dir_opt {
        Some(dir) => dir,
        None => state_dir()?,
    };
    let hook_token = std::env::var(ENV_HOOK_TOKEN).unwrap_or_default();
    if hook_token.is_empty() {
        warn!("AIDP_HOOK_TOKEN not set — inbound webhooks (/api/hooks/:id) are disabled");
    }
    let ui_token = std::env::var(ENV_UI_TOKEN).unwrap_or_default();
    check_lan_bind_token(bind, &ui_token)?;
    if ui_token.is_empty() {
        warn!("AIDP_UI_TOKEN not set — off-loopback callers get 401 on every route");
    }
    let read_only = READ_ONLY_TRUE_VALUES.contains(
        &std::env::var(ENV_READ_ONLY)
            .unwrap_or_default()
            .to_lowercase()
            .as_str(),
    );
    if read_only {
        info!("AIDP_READ_ONLY — POST/PUT/PATCH/DELETE refused outside /api/hooks");
    }
    if let Ok(raw) = std::env::var(ENV_MAX_BODY_BYTES) {
        match parse_positive_byte_count(&raw) {
            Some(n) => info!(
                "AIDP_MAX_BODY_BYTES={n} — request bodies above {n} bytes are refused with 413"
            ),
            None => warn!(
                "AIDP_MAX_BODY_BYTES={raw:?} is not a positive byte count — using the default {MAX_BODY_BYTES}"
            ),
        }
    }
    log_max_inflight_requests_override();
    let allowed_origins =
        parse_allowed_origins(&std::env::var(ENV_ALLOWED_ORIGINS).unwrap_or_default());
    if allowed_origins.is_empty() {
        info!(
            "AIDP_ALLOWED_ORIGINS not set — only the daemon's own origins (loopback, plus the LAN address when bound off-loopback; port {port}) may send mutating requests with an Origin; set it to also allow a dev server such as http://127.0.0.1:5173"
        );
    }
    let vault_root = aidp_core::vault::resolve_global()
        .map(|vault| vault.root)
        .unwrap_or_default();
    let config = HttpConfig {
        state_dir: dir.clone(),
        vault_root,
        hook_token,
        ui_token,
        read_only,
        allowed_origins,
    };
    let addr = socket_addr(bind, port);
    let listener = TcpListener::bind(addr).map_err(|e| anyhow!("cannot bind {addr}: {e}"))?;
    info!(addr = %addr, state = %dir.display(), "gateway listening");
    let environment = crate::models::runtime_environment::RuntimeEnvironment {
        skill_root: skill_root(),
        ui_dir: crate::static_ui::ui_dir_or_env(ui_dir),
        ..Default::default()
    };
    let handle = serve_on_with_environment(listener, config, environment)?;
    let (stop_tx, stop_rx) = channel::<()>();
    if let Err(e) = ctrlc::set_handler(move || {
        let _ = stop_tx.send(());
    }) {
        error!("cannot install the Ctrl-C handler: {e}");
    }
    wait_for_stop(&handle, &stop_rx);
    info!("stop requested — shutting down");
    handle.stop();
    Ok(())
}

/// Blocks until a stop signal arrives or the accept thread exits. A lost signal
/// channel (no Ctrl-C handler) never stops the daemon: it keeps polling the thread.
fn wait_for_stop(handle: &ServeHandle, signals: &Receiver<()>) {
    while !handle.is_finished() {
        match signals.recv_timeout(SIGNAL_POLL_INTERVAL) {
            Ok(()) => return,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => thread::sleep(SIGNAL_POLL_INTERVAL),
        }
    }
}

/// Logs a valid `AIDP_MAX_INFLIGHT_REQUESTS` override, or the fallback for an invalid one.
fn log_max_inflight_requests_override() {
    let Ok(raw) = std::env::var(MAX_INFLIGHT_REQUESTS_ENV) else {
        return;
    };
    match parse_positive_byte_count(&raw) {
        Some(n) => info!(
            "{MAX_INFLIGHT_REQUESTS_ENV}={n} — requests beyond {n} in flight are refused with 503"
        ),
        None => warn!(
            "{MAX_INFLIGHT_REQUESTS_ENV}={raw:?} is not a positive count — using the default {MAX_INFLIGHT_REQUESTS}"
        ),
    }
}

/// The daemon's own origins for the port it really listens on: the three loopback names, the
/// bound address when it is a specific non-loopback one, and `detected` (the machine's LAN
/// addresses) when it listens on `0.0.0.0` / `::`. A UI served by the daemon itself sends one
/// of these on its `POST`s. No wildcard, no `null`, no other port.
fn own_origins(port: u16, bound: IpAddr, detected: &[IpAddr]) -> Vec<String> {
    let mut origins = vec![
        format!("http://127.0.0.1:{port}"),
        format!("http://localhost:{port}"),
        format!("http://[::1]:{port}"),
    ];
    let lan: Vec<IpAddr> = if bound.is_unspecified() {
        detected.to_vec()
    } else {
        vec![bound]
    };
    for ip in lan {
        if ip.is_loopback() || ip.is_unspecified() || ip.is_multicast() {
            continue;
        }
        origins.push(match ip {
            IpAddr::V4(v4) => format!("http://{v4}:{port}"),
            IpAddr::V6(v6) => format!("http://[{v6}]:{port}"),
        });
    }
    origins
}

/// Adds the daemon's own origins to the configured list, without duplicates.
fn add_own_origins(allowed: &mut Vec<String>, port: u16, bound: IpAddr, detected: &[IpAddr]) {
    for origin in own_origins(port, bound, detected) {
        if !allowed.contains(&origin) {
            allowed.push(origin);
        }
    }
}

/// Local addresses used to reach the LAN: the source address the OS picks for an outbound
/// IPv4 and IPv6 route (a connected UDP socket, nothing is sent). Other interfaces are
/// covered by `AIDP_ALLOWED_ORIGINS`.
fn detect_lan_ips() -> Vec<IpAddr> {
    let probes: [(&str, &str); 2] = [("0.0.0.0:0", "192.0.2.1:9"), ("[::]:0", "[2001:db8::1]:9")];
    probes
        .iter()
        .filter_map(|(local, remote)| {
            let socket = std::net::UdpSocket::bind(local).ok()?;
            socket.connect(remote).ok()?;
            socket.local_addr().ok().map(|a| a.ip())
        })
        .filter(|ip| !ip.is_loopback() && !ip.is_unspecified())
        .collect()
}

/// Refuses a non-loopback bind unless `AIDP_UI_TOKEN` is at least [`MIN_LAN_TOKEN_LEN`] long.
fn check_lan_bind_token(bind: IpAddr, token: &str) -> anyhow::Result<()> {
    if !bind.is_loopback() && token.chars().count() < MIN_LAN_TOKEN_LEN {
        anyhow::bail!(
            "refusing to listen on {bind}: a non-loopback --bind needs AIDP_UI_TOKEN of at least {MIN_LAN_TOKEN_LEN} characters (it is {} now)",
            token.chars().count()
        );
    }
    Ok(())
}

/// `AIDP_ALLOWED_ORIGINS` split on `,`, trimmed, empties dropped.
fn parse_allowed_origins(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|o| !o.is_empty())
        .map(str::to_string)
        .collect()
}

fn hdr(k: &str, v: &str) -> Header {
    Header::from_bytes(k.as_bytes(), v.as_bytes()).expect(INVALID_HEADER_MESSAGE)
}

fn header_value(req: &Request, name: &str) -> String {
    req.headers()
        .iter()
        .find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(name))
        .map(|h| h.value.as_str().to_string())
        .unwrap_or_default()
}

/// Methods whose body carries a JSON payload the route handlers need to read.
fn method_has_body(m: &Method) -> bool {
    matches!(m, Method::Post | Method::Put | Method::Patch)
}

/// Kept in one place so the CORS preflight advertises exactly the methods `route()` accepts.
const CORS_METHODS: &str = "GET, POST, PUT, PATCH, DELETE, OPTIONS";

/// Methods that change state. One definition shared by read-only mode and the Origin check
/// so the two notions cannot diverge.
fn is_mutating(m: &Method) -> bool {
    matches!(
        m,
        Method::Post | Method::Put | Method::Patch | Method::Delete
    )
}

/// The request's `Origin` header (empty when absent) and the allowlist it is checked against.
#[derive(Clone, Copy)]
struct OriginCheck<'a> {
    origin: &'a str,
    allowed: &'a [String],
}

impl OriginCheck<'_> {
    fn is_allowed(&self) -> bool {
        !self.origin.is_empty() && self.allowed.iter().any(|o| o == self.origin)
    }
}

/// The central auth and read-only decision, a pure function. Order: OPTIONS, `/hooks`
/// exemption, read-only, Origin check, then loopback or token.
#[derive(Debug, PartialEq)]
enum Gate {
    Allow,
    Unauthorized,
    ReadOnly,
    ForbiddenOrigin,
}

#[allow(clippy::too_many_arguments)]
fn gate(
    origin: OriginCheck<'_>,
    is_loopback: bool,
    ui_token: &str,
    given: &str,
    read_only: bool,
    m: &Method,
    segs: &[&str],
) -> Gate {
    if *m == Method::Options {
        return Gate::Allow;
    }
    if segs.first() == Some(&SEGMENT_HOOKS) {
        return Gate::Allow;
    }
    if read_only && is_mutating(m) {
        return Gate::ReadOnly;
    }
    if is_mutating(m) && !origin.origin.is_empty() && !origin.is_allowed() {
        return Gate::ForbiddenOrigin;
    }
    if !is_loopback && (ui_token.is_empty() || !ct_eq(given, ui_token)) {
        return Gate::Unauthorized;
    }
    Gate::Allow
}

/// Whether a body of `declared_len` bytes may be read; `None` is accepted as it is also
/// the chunked case, where the `take()` in `read_body()` stays the reference bound.
#[derive(Debug, PartialEq)]
enum BodyBound {
    Accept,
    TooLarge,
}

fn bound_body(declared_len: Option<usize>, max: usize) -> BodyBound {
    match declared_len {
        Some(n) if n > max => BodyBound::TooLarge,
        _ => BodyBound::Accept,
    }
}

/// Reads what is left of the body through a fixed buffer, required before every respond:
/// dropping an unread request allocates the size the client announced.
fn drain_body(req: &mut Request) {
    let _ = std::io::copy(req.as_reader(), &mut std::io::sink());
}

fn too_large(max: usize) -> Reply {
    err(
        HTTP_PAYLOAD_TOO_LARGE,
        format!("request body too large: limit is {max} bytes"),
    )
}

/// The JSON body of an allowed request, or the reply refusing it. An empty body is `{}`;
/// a non-empty unparsable one is a `400`.
fn read_body(req: &mut Request, method: &Method, max: usize) -> Result<(Value, Vec<u8>), Reply> {
    if bound_body(req.body_length(), max) == BodyBound::TooLarge {
        return Err(too_large(max));
    }
    let capacity = req
        .body_length()
        .map(|n| n.min(max))
        .unwrap_or(DEFAULT_BODY_CAPACITY);
    let mut buf = Vec::with_capacity(capacity);
    let _ = req
        .as_reader()
        .take((max as u64).saturating_add(1))
        .read_to_end(&mut buf);
    if bound_body(Some(buf.len()), max) == BodyBound::TooLarge {
        return Err(too_large(max));
    }
    if !method_has_body(method) {
        return Ok((json!({}), buf));
    }
    let Ok(text) = std::str::from_utf8(&buf) else {
        return Err(err(HTTP_BAD_REQUEST, INVALID_JSON_MESSAGE));
    };
    if text.trim().is_empty() {
        return Ok((json!({}), buf));
    }
    let v = serde_json::from_str(text).map_err(|_| err(HTTP_BAD_REQUEST, INVALID_JSON_MESSAGE))?;
    Ok((v, buf))
}

/// Mint a single-use SSE token, evicting expired ones first.
fn mint_sse_token(app: &App) -> String {
    let t = crate::ids::new_id().replace('-', "");
    let mut m = crate::sync::lock(&app.sse_tokens);
    m.retain(|_, at| at.elapsed() <= SSE_TOKEN_TTL);
    m.insert(t.clone(), Instant::now());
    t
}

/// Consume an SSE token: true when it existed and was still fresh. Always single use.
fn consume_sse_token(app: &App, t: &str) -> bool {
    let mut m = crate::sync::lock(&app.sse_tokens);
    m.retain(|_, at| at.elapsed() <= SSE_TOKEN_TTL);
    m.remove(t).is_some()
}

/// Request credentials handed to `route()`: the header tokens, GitHub's signature header,
/// and the raw body bytes the signature is computed over.
struct ReqAuth<'a> {
    auth: &'a str,
    xtoken: &'a str,
    hub_sig: &'a str,
    github_event: &'a str,
    raw_body: &'a [u8],
    /// `Accept-Encoding`, for the precompressed files of the web UI.
    accept_encoding: &'a str,
}

impl ReqAuth<'_> {
    /// The bearer token: `Authorization: Bearer`, else `X-Aidp-Token`. Never the query string.
    fn bearer(&self) -> String {
        self.auth
            .strip_prefix(BEARER_PREFIX)
            .map(|s| s.to_string())
            .unwrap_or_else(|| self.xtoken.to_string())
    }
}

fn handle(app: Arc<App>, mut req: Request) {
    let started = Instant::now();
    let method = req.method().clone();
    let url = req.url().to_string();
    let logged_path = loggable_path(&url);
    let span = info_span!(
        "request",
        method = %method,
        path = %logged_path,
    );
    let _entered = span.enter();
    let (path, query) = match url.split_once('?') {
        Some((p, q)) => (p.to_string(), q.to_string()),
        None => (url.clone(), String::new()),
    };
    let auth = header_value(&req, HEADER_AUTHORIZATION);
    let xtoken = header_value(&req, HEADER_AIDP_TOKEN);
    let origin_hdr = header_value(&req, HEADER_ORIGIN);
    let hub_sig = header_value(&req, HEADER_HUB_SIGNATURE);
    let github_event = header_value(&req, HEADER_GITHUB_EVENT);
    let accept_encoding = header_value(
        &req,
        crate::models::constants::static_ui::HEADER_ACCEPT_ENCODING,
    );
    let origin = OriginCheck {
        origin: &origin_hdr,
        allowed: &app.allowed_origins,
    };
    let origin_allowed = origin.is_allowed();

    let is_loopback = req
        .remote_addr()
        .map(|a| a.ip().is_loopback() && !app.require_loopback_token)
        .unwrap_or(false);
    let p_for_gate = path.trim_start_matches("/api").trim_matches('/');
    let segs_for_gate: Vec<&str> = if p_for_gate.is_empty() {
        vec![]
    } else {
        p_for_gate.split('/').collect()
    };
    let query_for_gate = parse_query(&query);
    let mut given = auth
        .strip_prefix(BEARER_PREFIX)
        .map(|s| s.to_string())
        .unwrap_or_else(|| xtoken.to_string());
    let mut es_invalid = false;
    if method == Method::Get && segs_for_gate.as_slice() == [SEGMENT_EVENTS] {
        if let Some(es) = query_for_gate.get(QUERY_ES).filter(|v| !v.is_empty()) {
            if consume_sse_token(&app, es) {
                given = app.ui_token.clone();
            } else {
                es_invalid = !is_loopback;
            }
        }
    }

    let confidential_configuration = segs_for_gate.as_slice()
        == [
            crate::models::constants::mcp_configuration::RUNTIME_SEGMENT,
            crate::models::constants::mcp_configuration::CONFIGURATIONS_SEGMENT,
        ];
    // The UI build holds no data: it is served without a token, off-loopback too. Every
    // `/api/*` route keeps the gate below.
    let static_public = matches!(method, Method::Get | Method::Head)
        && app.static_ui.root().is_some()
        && !crate::static_ui::is_api_path(&path);
    let decision = if static_public {
        Gate::Allow
    } else if es_invalid
        || (confidential_configuration
            && (app.ui_token.is_empty() || !ct_eq(&given, &app.ui_token)))
    {
        Gate::Unauthorized
    } else {
        gate(
            origin,
            is_loopback,
            &app.ui_token,
            &given,
            app.read_only,
            &method,
            &segs_for_gate,
        )
    };
    let reply = match decision {
        Gate::ForbiddenOrigin => {
            drain_body(&mut req);
            err(HTTP_FORBIDDEN, FORBIDDEN_ORIGIN_MESSAGE)
        }
        Gate::Unauthorized => {
            drain_body(&mut req);
            err(HTTP_UNAUTHORIZED, UNAUTHORIZED_MESSAGE)
        }
        Gate::ReadOnly => {
            drain_body(&mut req);
            err(HTTP_FORBIDDEN, READ_ONLY_MESSAGE)
        }
        Gate::Allow => {
            let body = read_body(&mut req, &method, app.max_body_bytes);
            drain_body(&mut req);
            match body {
                Err(r) => r,
                Ok((body_json, raw)) => {
                    let creds = ReqAuth {
                        auth: &auth,
                        xtoken: &xtoken,
                        hub_sig: &hub_sig,
                        github_event: &github_event,
                        raw_body: &raw,
                        accept_encoding: &accept_encoding,
                    };
                    match route(&app, &method, &path, &query, body_json, &creds) {
                        Ok(r) => r,
                        Err(e) => api_err(e),
                    }
                }
            }
        }
    };
    record_access(&reply, started);
    let mut cors: Vec<Header> = Vec::new();
    if origin_allowed {
        cors.push(hdr(HEADER_ALLOW_ORIGIN, &origin_hdr));
        cors.push(hdr(HEADER_VARY, HEADER_ORIGIN));
    }
    if origin_hdr.is_empty() || origin_allowed {
        cors.push(hdr(HEADER_ALLOW_HEADERS, CORS_ALLOWED_HEADERS));
        cors.push(hdr(HEADER_ALLOW_METHODS, CORS_METHODS));
    }
    match reply {
        Reply::Json(code, v) => {
            let mut r = Response::from_string(v.to_string()).with_status_code(StatusCode(code));
            r.add_header(hdr(HEADER_CONTENT_TYPE, CONTENT_TYPE_JSON));
            for h in cors {
                r.add_header(h);
            }
            let _ = req.respond(r);
        }
        Reply::Empty(code) => {
            let mut r = Response::empty(StatusCode(code));
            for h in cors {
                r.add_header(h);
            }
            let _ = req.respond(r);
        }
        Reply::Text(code, body, content_type) => {
            let mut r = Response::from_string(body).with_status_code(StatusCode(code));
            r.add_header(hdr(HEADER_CONTENT_TYPE, content_type));
            for h in cors {
                r.add_header(h);
            }
            let _ = req.respond(r);
        }
        Reply::Asset(asset) => {
            let mut r = Response::from_data(asset.bytes).with_status_code(StatusCode(HTTP_OK));
            r.add_header(hdr(HEADER_CONTENT_TYPE, &asset.content_type));
            r.add_header(hdr(HEADER_CACHE_CONTROL, CACHE_PRIVATE_DAY));
            r.add_header(hdr(HEADER_CONTENT_SECURITY_POLICY, CSP_SANDBOX));
            r.add_header(hdr(HEADER_CONTENT_TYPE_OPTIONS, NOSNIFF));
            for h in cors {
                r.add_header(h);
            }
            let _ = req.respond(r);
        }
        Reply::Static(file) => {
            let mut r = Response::from_data(file.bytes).with_status_code(StatusCode(HTTP_OK));
            r.add_header(hdr(HEADER_CONTENT_TYPE, file.content_type));
            r.add_header(hdr(HEADER_CACHE_CONTROL, file.cache_control));
            r.add_header(hdr(HEADER_CONTENT_TYPE_OPTIONS, NOSNIFF));
            r.add_header(hdr(
                HEADER_VARY,
                crate::models::constants::static_ui::HEADER_ACCEPT_ENCODING,
            ));
            if let Some(encoding) = file.content_encoding {
                r.add_header(hdr(
                    crate::models::constants::static_ui::HEADER_CONTENT_ENCODING,
                    encoding.token(),
                ));
            }
            let _ = req.respond(r);
        }
        Reply::Stream(reader) => {
            let mut headers = vec![
                hdr(HEADER_CONTENT_TYPE, CONTENT_TYPE_EVENT_STREAM),
                hdr(HEADER_CACHE_CONTROL, CACHE_NO_CACHE),
            ];
            headers.extend(cors);
            let r = Response::new(StatusCode(HTTP_OK), headers, reader, None, None);
            let _ = req.respond(r);
        }
    }
}

fn reply_status(reply: &Reply) -> u16 {
    match reply {
        Reply::Json(code, _) | Reply::Empty(code) | Reply::Text(code, _, _) => *code,
        Reply::Stream(_) | Reply::Asset(_) | Reply::Static(_) => HTTP_OK,
    }
}

/// Emits the access-log event with status and duration.
fn record_access(reply: &Reply, started: Instant) {
    let status = reply_status(reply);
    let duration_ms = started.elapsed().as_millis() as u64;
    info!(status, duration_ms, "request handled");
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Percent-decoding that is total on every input: the two digits after `%` are read
/// as raw bytes, never by slicing the `&str`.
fn urldecode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < bytes.len() => match (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                (Some(hi), Some(lo)) => {
                    out.push(hi * 16 + lo);
                    i += 2;
                }
                _ => out.push(b'%'),
            },
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

fn parse_query(q: &str) -> HashMap<String, String> {
    q.split('&')
        .filter(|kv| !kv.is_empty())
        .map(|kv| match kv.split_once('=') {
            Some((k, v)) => (urldecode(k), urldecode(v)),
            None => (urldecode(kv), String::new()),
        })
        .collect()
}

fn status_json(app: &App) -> Value {
    let all = app.sessions.list(None, None, 100_000);
    let autos = app.automations.list();
    json!({
        "uptimeSec": (now_ms() - app.started_ms) / 1000,
        "startedAt": app.started_at,
        "sessions": { "running": app.sessions.running_count(), "queued": app.sessions.queue_depth(), "total": all.len() },
        "automations": { "enabled": autos.iter().filter(|a| a.enabled).count(), "total": autos.len() },
        "hooksEnabled": !app.hook_token.is_empty(),
        "skillRoot": app.skill_root.as_ref().map(|p| p.display().to_string()),
        "aidpBin": app.sessions.runtime_environment().get(crate::models::constants::flow_execution::AIDP_BIN_ENV).cloned().or_else(|| std::env::current_exe().ok().map(|path| path.display().to_string())),
        "vault": (!app.vault_root.as_os_str().is_empty()).then(|| app.vault_root.display().to_string()),
        "stateDir": app.state_dir.display().to_string(),
        "rateLimit": app.sessions.rate_limit(),
        crate::models::constants::runtime_api::CAPABILITIES: crate::models::gateway_capabilities::GatewayCapabilities::CURRENT,
    })
}

fn health_payload(app: &App) -> Value {
    let mut v =
        crate::health::health_json(&app.state_dir, &app.sessions, !app.hook_token.is_empty());
    v["auth"] = json!({
        "uiTokenConfigured": !app.ui_token.is_empty(),
        "readOnly": app.read_only,
    });
    v["threads"]["requestsInFlight"] = json!(app.inflight_requests.load(Ordering::SeqCst));
    v["threads"]["requestsCeiling"] = json!(app.max_inflight_requests);
    v
}

/// What became of one frame pushed into a bounded SSE channel.
#[derive(Debug, PartialEq, Eq)]
enum SendOutcome {
    Sent,
    Stalled,
    Closed,
}

/// Pushes `frame`, retrying a full channel for up to [`SSE_STALL_GRACE`].
fn send_frame(tx: &SyncSender<Vec<u8>>, frame: Vec<u8>) -> SendOutcome {
    send_frame_with(tx, frame, SSE_STALL_GRACE)
}

/// [`send_frame`] with an explicit grace: a channel still full once `grace`
/// has elapsed means a stalled client; a dropped receiver means a closed one.
fn send_frame_with(tx: &SyncSender<Vec<u8>>, frame: Vec<u8>, grace: Duration) -> SendOutcome {
    let deadline = Instant::now() + grace;
    let mut pending = frame;
    loop {
        match tx.try_send(pending) {
            Ok(()) => return SendOutcome::Sent,
            Err(TrySendError::Disconnected(_)) => return SendOutcome::Closed,
            Err(TrySendError::Full(returned)) => {
                let now = Instant::now();
                if now >= deadline {
                    return SendOutcome::Stalled;
                }
                pending = returned;
                thread::sleep(SSE_RETRY_INTERVAL.min(deadline - now));
            }
        }
    }
}

/// Forwards one frame; false means the stream must end. A stalled client
/// raises exactly one [`SSE_OVERFLOW_EVENT`] before its stream is closed.
fn forward_frame(
    app: &App,
    tx: &SyncSender<Vec<u8>>,
    frame: Vec<u8>,
    stream: &str,
    session_id: Option<&str>,
) -> bool {
    match send_frame(tx, frame) {
        SendOutcome::Sent => true,
        SendOutcome::Closed => false,
        SendOutcome::Stalled => {
            app.bus.emit(
                SSE_OVERFLOW_EVENT,
                json!({ "stream": stream, "sessionId": session_id, "capacity": SSE_CHANNEL_CAPACITY }),
            );
            false
        }
    }
}

/// A `Read` fed by a channel — the body of a streaming (SSE) response.
struct ChanReader {
    rx: Receiver<Vec<u8>>,
    buf: Vec<u8>,
    pos: usize,
}

/// Size of the chunks tiny_http emits; each batch is padded with an SSE comment to
/// `k*8192 + 1` bytes so live frames are pushed out at once.
const SSE_CHUNK: usize = 8192;

fn pad_sse_batch(batch: &mut Vec<u8>) {
    let mut target = (batch.len() / SSE_CHUNK + 1) * SSE_CHUNK + 1;
    if target < batch.len() + 3 {
        target += SSE_CHUNK;
    }
    batch.push(b':');
    batch.resize(target - 2, b' ');
    batch.extend_from_slice(b"\n\n");
}

impl Read for ChanReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if self.pos >= self.buf.len() {
            let mut batch = match self.rx.recv() {
                Ok(b) => b,
                Err(_) => return Ok(0),
            };
            while let Ok(more) = self.rx.try_recv() {
                batch.extend_from_slice(&more);
            }
            pad_sse_batch(&mut batch);
            self.buf = batch;
            self.pos = 0;
        }
        let n = out.len().min(self.buf.len() - self.pos);
        out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

fn sse(v: &Value) -> Vec<u8> {
    format!("data: {v}\n\n").into_bytes()
}

fn sse_ping() -> Vec<u8> {
    b": ping\n\n".to_vec()
}

/// Replays the recent bus events, then follows the bus, sending a connect frame first
/// so `EventSource` fires `open`.
fn event_stream(app: &Arc<App>, since: u64) -> ChanReader {
    let (tx, rx) = sync_channel::<Vec<u8>>(SSE_CHANNEL_CAPACITY);
    let sub = app.bus.subscribe();
    let recent = app.bus.recent(100, since);
    let app = app.clone();
    thread::spawn(move || {
        let forward = |frame: Vec<u8>| forward_frame(&app, &tx, frame, SSE_STREAM_EVENTS, None);
        if !forward(b": connected\n\n".to_vec()) {
            return;
        }
        for ev in recent {
            if !forward(sse(&ev)) {
                return;
            }
        }
        loop {
            let frame = match sub.recv_timeout(Duration::from_secs(25)) {
                Ok(ev) => sse(&ev),
                Err(RecvTimeoutError::Timeout) => sse_ping(),
                Err(RecvTimeoutError::Disconnected) => return,
            };
            if !forward(frame) {
                return;
            }
        }
    });
    ChanReader {
        rx,
        buf: Vec::new(),
        pos: 0,
    }
}

/// Replays persisted events, then follows; ends with `{type:"done"}` once the session is over.
fn session_stream(app: &Arc<App>, id: &str) -> ChanReader {
    let (tx, rx) = sync_channel::<Vec<u8>>(SSE_CHANNEL_CAPACITY);
    let app = app.clone();
    let id = id.to_string();
    thread::spawn(move || {
        let forward =
            |frame: Vec<u8>| forward_frame(&app, &tx, frame, SSE_STREAM_SESSION, Some(id.as_str()));
        if !forward(b": connected\n\n".to_vec()) {
            return;
        }
        let mut sent = 0usize;
        loop {
            let evs = app.sessions.events(&id);
            while sent < evs.len() {
                if !forward(sse(&json!({ "type": "event", "event": evs[sent] }))) {
                    return;
                }
                sent += 1;
            }
            match app.sessions.get(&id) {
                Some(rec) if rec.status == SessionStatus::Running => {
                    thread::sleep(Duration::from_millis(250))
                }
                Some(rec) => {
                    forward(sse(
                        &json!({ "type": "done", "status": rec.status, "exitCode": rec.exit_code }),
                    ));
                    return;
                }
                None => return,
            }
        }
    });
    ChanReader {
        rx,
        buf: Vec::new(),
        pos: 0,
    }
}

fn res_reply(r: anyhow::Result<()>) -> Reply {
    match r {
        Ok(()) => ok(),
        Err(e) => api_err(e),
    }
}

fn resume(app: &Arc<App>, id: &str, body: &Value) -> Reply {
    let Some(parent) = app.sessions.get(id) else {
        return err(HTTP_NOT_FOUND, SESSION_NOT_FOUND);
    };
    let prompt = body["prompt"]
        .as_str()
        .filter(|p| !p.trim().is_empty())
        .unwrap_or(DEFAULT_RESUME_PROMPT);
    let mut origin: Origin =
        serde_json::from_value(body["origin"].clone()).unwrap_or_else(|_| Origin::new(ORIGIN_API));
    origin.r#ref = Some(parent.id.clone());
    origin.label = Some(
        format!("resume {}", parent.template_id.clone().unwrap_or_default())
            .trim()
            .to_string(),
    );
    let auto_close = body["autoClose"]
        .as_bool()
        .unwrap_or(origin.kind != ORIGIN_UI);
    let req = StartRequest {
        project_id: parent.project_id.clone(),
        engine: Some(ENGINE_PROMPT.to_string()),
        prompt: Some(prompt.to_string()),
        permission_mode: Some(
            body["permissionMode"]
                .as_str()
                .map(|s| s.to_string())
                .unwrap_or(parent.permission_mode.clone()),
        ),
        model: parent.model.clone(),
        max_budget_usd: body["maxBudgetUsd"].as_f64().or(parent.max_budget_usd),
        resume_from: Some(parent.id.clone()),
        origin: Some(origin),
        auto_close: Some(auto_close),
        ..Default::default()
    };
    match app.sessions.start(req) {
        Ok(l) => Reply::Json(HTTP_OK, l.to_json()),
        Err(e) => api_err(e),
    }
}

/// Parses a ticket status in kebab-case with separator variants, matching what
/// `aidp ticket list --status` accepts.
fn parse_ticket_status(s: &str) -> anyhow::Result<Status> {
    match s.trim().to_ascii_lowercase().as_str() {
        TICKET_TODO => Ok(Status::Todo),
        TICKET_QUEUED => Ok(Status::Queued),
        TICKET_IN_PROGRESS | TICKET_IN_PROGRESS_SNAKE | TICKET_IN_PROGRESS_FLAT => Ok(Status::InProgress),
        TICKET_IN_REVIEW | TICKET_IN_REVIEW_SNAKE | TICKET_IN_REVIEW_FLAT => Ok(Status::InReview),
        TICKET_DONE => Ok(Status::Done),
        TICKET_BLOCKED => Ok(Status::Blocked),
        TICKET_CANCELLED | TICKET_CANCELED_US => Ok(Status::Cancelled),
        _ => anyhow::bail!(
            "unknown status `{s}`: expected todo/queued/in-progress/in-review/done/blocked/cancelled"
        ),
    }
}

/// Mirrors `aidp-tickets::cli`'s own private `parse_source`.
fn parse_ticket_source(s: &str) -> anyhow::Result<Source> {
    match s.trim().to_ascii_lowercase().as_str() {
        SOURCE_GITHUB => Ok(Source::Github),
        SOURCE_LOCAL => Ok(Source::Local),
        _ => anyhow::bail!("unknown source `{s}`: expected github/local"),
    }
}

/// Opens the ticket store for a route handler, once a vault root is in hand.
/// See [`ticket_store_or_503`] for the `503` answered when there is none.
fn open_ticket_store(vault_root: &str) -> anyhow::Result<TicketStore> {
    TicketStore::open(Path::new(vault_root))
}

/// Resolves the configured vault, answering `503` when there is none, then opens the
/// ticket store. A macro because it must `return` out of `route()`.
macro_rules! ticket_store_or_503 {
    ($app:expr) => {{
        if $app.vault_root.as_os_str().is_empty() {
            return Ok(err(SERVICE_UNAVAILABLE, NO_VAULT_MESSAGE));
        }
        open_ticket_store(&$app.vault_root.display().to_string())?
    }};
}

/// One row of `GET /api/worktrees`.
#[derive(Serialize, Clone)]
struct WorktreeRow {
    key: String,
    project: String,
    path: String,
    branch: Option<String>,
    exists: bool,
    modified: usize,
    prunable: bool,
}

/// The body of `GET /api/tickets/:key/worktree`.
#[derive(Serialize)]
struct TicketWorktree {
    path: String,
    branch: String,
    exists: bool,
    modified: usize,
}

/// `GET /api/worktrees`: the worktrees under `<vault>/Pipelines`, optionally of one project.
/// `prunable` is true when the owning ticket is done, cancelled or unknown.
fn list_worktrees(
    projects: &[Project],
    store: &TicketStore,
    project_filter: Option<&str>,
) -> Vec<WorktreeRow> {
    let statuses: HashMap<String, Status> = store
        .list(&ListFilter {
            archived: true,
            ..ListFilter::default()
        })
        .into_iter()
        .map(|ticket| (ticket.key, ticket.status))
        .collect();
    let runner = OsProcessRunner;
    let mut listed = Vec::new();
    for project in projects {
        if project_filter.is_some_and(|wanted| wanted != project.id) {
            continue;
        }
        for entry in crate::worktrees::list(&runner, Path::new(&project.path), store.vault_root()) {
            if let Some(key) = crate::worktrees::worktree_owner(&entry.path) {
                listed.push((project, key, entry));
            }
        }
    }
    std::thread::scope(|scope| {
        let workers: Vec<_> = listed
            .into_iter()
            .map(|(project, key, entry)| {
                let (runner, statuses) = (&runner, &statuses);
                scope.spawn(move || {
                    let prunable = matches!(
                        statuses.get(&key),
                        None | Some(Status::Done) | Some(Status::Cancelled)
                    );
                    let exists = entry.path.exists();
                    let modified = if exists {
                        crate::worktrees::modified_count(runner, &entry.path)
                    } else {
                        0
                    };
                    WorktreeRow {
                        key,
                        project: project.id.clone(),
                        path: entry.path.display().to_string(),
                        branch: entry.branch,
                        exists,
                        modified,
                        prunable,
                    }
                })
            })
            .collect();
        workers
            .into_iter()
            .filter_map(|worker| worker.join().ok())
            .collect()
    })
}

/// `list_worktrees` answered from the stale-while-revalidate cache of the app.
fn cached_worktrees(app: &App, project_filter: Option<String>) -> anyhow::Result<Value> {
    let projects = app.projects.list();
    let vault = app.vault_root.clone();
    let slot = project_filter.clone().unwrap_or_default();
    app.route_caches.worktrees.get(slot, move || {
        let store = TicketStore::open(&vault)?;
        Ok(json!(list_worktrees(
            &projects,
            &store,
            project_filter.as_deref()
        )))
    })
}

/// The project of a ticket and the repo segment its worktrees live under.
fn ticket_project(
    app: &App,
    store: &TicketStore,
    key: &str,
) -> Result<(Project, String), ApiError> {
    let local = store.get(key).ok().map(|(ticket, _, _)| ticket.service);
    let external = app
        .flows
        .as_ref()
        .and_then(|engine| engine.get(key).ok())
        .and_then(|state| {
            crate::flows::ticket_snapshot::project_name(&state.ticket).map(str::to_string)
        });
    let project_name = local.or(external).ok_or_else(|| ApiError {
        status: ApiStatus::NotFound,
        message: format!("{TICKET_NOT_FOUND_MESSAGE} {key}"),
    })?;
    let project = app.projects.get(&project_name).ok_or_else(|| ApiError {
        status: ApiStatus::NotFound,
        message: format!("{UNKNOWN_PROJECT_MESSAGE} {key}"),
    })?;
    let repo = aidp_core::vault::repo_segment(Path::new(&project.path));
    Ok((project, repo))
}

/// Worktrees the project holds under the vault that exist on disk.
fn held_worktrees(project: &Project, vault: &Path) -> usize {
    crate::worktrees::list(&OsProcessRunner, Path::new(&project.path), vault)
        .iter()
        .filter(|entry| {
            crate::worktrees::worktree_owner(&entry.path).is_some() && entry.path.exists()
        })
        .count()
}

fn ensure_error(error: crate::worktrees::EnsureError) -> ApiError {
    let status = match error {
        crate::worktrees::EnsureError::BranchCheckedOutElsewhere { .. } => ApiStatus::Conflict,
        crate::worktrees::EnsureError::InvalidKey(_) => ApiStatus::BadRequest,
        crate::worktrees::EnsureError::Git(_) => ApiStatus::Upstream,
        crate::worktrees::EnsureError::Io(_) => ApiStatus::Io,
    };
    ApiError {
        status,
        message: error.to_string(),
    }
}

fn remove_error(error: crate::worktrees::RemoveError) -> ApiError {
    let status = match error {
        crate::worktrees::RemoveError::InvalidKey(_) => ApiStatus::BadRequest,
        crate::worktrees::RemoveError::Git(_) => ApiStatus::Upstream,
        crate::worktrees::RemoveError::Locked { .. } => ApiStatus::Conflict,
    };
    ApiError {
        status,
        message: error.to_string(),
    }
}

/// `POST /api/worktrees/ensure`: creates or reuses the ticket worktree.
/// Past the project's `caps.worktrees` the reply asks the step to wait instead.
fn ensure_worktree(
    app: &App,
    store: &TicketStore,
    key: &str,
) -> Result<WorktreeEnsureResponse, ApiError> {
    let (project, repo) = ticket_project(app, store, key)?;
    let project_path = Path::new(&project.path);
    let vault = store.vault_root();
    let path = crate::worktrees::worktree_path(vault, &repo, key);
    if !path.exists() {
        let config =
            aidp_integrations::flow_repo::load(project_path).map_err(|error| ApiError {
                status: ApiStatus::Config,
                message: error.to_string(),
            })?;
        if held_worktrees(&project, vault) >= config.caps.worktrees as usize {
            return Ok(WorktreeEnsureResponse {
                path: None,
                base_sha: None,
                reused: false,
                waiting_external: true,
                reason: Some(WORKTREE_CAP_REASON.to_string()),
            });
        }
    }
    let copy: Vec<String> = crate::worktrees::DEFAULT_COPY
        .iter()
        .map(|pattern| pattern.to_string())
        .collect();
    let base = crate::worktrees::ticket_base(store, key);
    let ensured = crate::worktrees::ensure_based(
        &OsProcessRunner,
        project_path,
        vault,
        &repo,
        key,
        &copy,
        base.as_deref(),
    )
    .map_err(ensure_error)?;
    let base_sha = crate::worktrees::base_sha_from(
        &OsProcessRunner,
        project_path,
        &ensured.workspace.path,
        base.as_deref(),
    )
    .map_err(|message| ApiError {
        status: ApiStatus::Upstream,
        message,
    })?;
    Ok(WorktreeEnsureResponse {
        path: Some(ensured.workspace.path.display().to_string()),
        base_sha: Some(base_sha),
        reused: ensured.reused,
        waiting_external: false,
        reason: None,
    })
}

/// `POST /api/worktrees/remove`: commits unsaved work on the branch, then deletes the worktree.
fn remove_ticket_worktree(
    app: &App,
    store: &TicketStore,
    key: &str,
) -> Result<WorktreeRemoveResponse, ApiError> {
    let (project, repo) = ticket_project(app, store, key)?;
    let path = crate::worktrees::worktree_path(store.vault_root(), &repo, key);
    if !path.exists() {
        return Ok(WorktreeRemoveResponse {
            removed: false,
            salvaged_ref: None,
        });
    }
    let removed = crate::worktrees::remove(
        &OsProcessRunner,
        &thread::sleep,
        Path::new(&project.path),
        &path,
        key,
    )
    .map_err(remove_error)?;
    Ok(WorktreeRemoveResponse {
        removed: true,
        salvaged_ref: removed.salvaged_ref,
    })
}

/// Reads the ticket key of a worktree route body.
fn worktree_key(body: Value) -> Result<String, ApiError> {
    serde_json::from_value::<WorktreeKeyRequest>(body)
        .map(|request| request.key)
        .map_err(|error| ApiError {
            status: ApiStatus::BadRequest,
            message: error.to_string(),
        })
}

/// `GET /api/tickets/:key/worktree`: where the ticket's worktree is, whether it exists and
/// how many paths it has modified.
fn ticket_worktree(app: &App, store: &TicketStore, ticket: &Ticket) -> TicketWorktree {
    let repo = app
        .projects
        .get(&ticket.service)
        .map(|project| aidp_core::vault::repo_segment(Path::new(&project.path)))
        .unwrap_or_else(|| ticket.service.clone());
    let path = crate::worktrees::worktree_path(store.vault_root(), &repo, &ticket.key);
    let exists = path.exists();
    let modified = if exists {
        crate::worktrees::modified_count(&OsProcessRunner, &path)
    } else {
        0
    };
    TicketWorktree {
        path: path.display().to_string(),
        branch: crate::worktrees::branch_for(&ticket.key),
        exists,
        modified,
    }
}

/// Creates a local ticket from a request body, over the public store and note API,
/// as `aidp ticket new` does from its flags.
fn create_local_ticket(store: &TicketStore, body: &Value) -> anyhow::Result<Value> {
    let service = body["service"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!(SERVICE_REQUIRED_MESSAGE))?;
    let summary = body["summary"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!(SUMMARY_REQUIRED_MESSAGE))?;
    let allow_unknown_service = body["allowUnknownService"].as_bool().unwrap_or(false);
    TicketStore::check_service(store.vault_root(), service, allow_unknown_service)?;

    let issue_type = match body["type"].as_str() {
        Some(s) => s.parse::<IssueType>()?,
        None => IssueType::Task,
    };
    let priority = match body["priority"].as_str() {
        Some(s) => s.parse::<Priority>()?,
        None => Priority::Medium,
    };

    let key = store.allocate_key()?;
    let template = body["template"]
        .as_str()
        .map(str::to_string)
        .or_else(|| store.routing().template_for(service).map(str::to_string));
    let track = body[TRACK_FIELD]
        .as_str()
        .map(str::parse::<Track>)
        .transpose()?;
    let epic = body["epic"].as_str().map(str::to_string);
    let labels: Vec<String> = body["labels"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    let raw_body = body["body"]
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| note::default_body(&key, summary));
    let ticket_body = note::normalise_body(&raw_body);

    let now = SystemClock.now_utc();
    let mut ticket = Ticket {
        key: key.clone(),
        source: Source::Local,
        r#type: issue_type,
        summary: summary.to_string(),
        status: Status::Todo,
        priority,
        priority_rank: 0,
        service: service.to_string(),
        template,
        track,
        labels,
        components: Vec::new(),
        epic,
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

    let path = store.save(&mut ticket, &ticket_body, &serde_yaml::Mapping::new())?;
    ledger::append(
        store.vault_root(),
        &Event {
            ts: SystemClock.now_utc(),
            key: key.clone(),
            from: None,
            to: Status::Todo,
            actor: Actor::Cli,
            reason: None,
            session_id: None,
        },
    );

    Ok(json!({ "key": key, "path": path.display().to_string() }))
}

/// Moves a ticket note to a service and, when a template is given, saves it once more,
/// as `aidp-tickets::cli` does.
fn route_ticket_http(
    store: &TicketStore,
    key: &str,
    service: &str,
    template: Option<&str>,
) -> anyhow::Result<Value> {
    TicketStore::check_service(store.vault_root(), service, false)?;
    let path = store.move_to_service(key, service)?;
    let path = match template {
        Some(t) => {
            let (mut ticket, body, extra) = store.get(key)?;
            ticket.template = Some(t.to_string());
            store.save(&mut ticket, &body, &extra)?
        }
        None => path,
    };
    Ok(json!({ "key": key, "path": path.display().to_string() }))
}

/// Guards, applies and journals one status transition as `Actor::Human`, mirroring
/// `aidp-tickets::cli`.
fn transition_ticket_http(
    store: &TicketStore,
    bus: &Bus,
    key: &str,
    to: Status,
    reason: Option<String>,
    force: bool,
) -> anyhow::Result<Value> {
    let (mut ticket, body, extra) = store.get(key)?;
    let from = ticket.status;
    model::guard(from, to, Actor::Human, force)?;
    ticket.status = to;
    let path = store.save(&mut ticket, &body, &extra)?;
    ledger::append(
        store.vault_root(),
        &Event {
            ts: SystemClock.now_utc(),
            key: key.to_string(),
            from: Some(from),
            to,
            actor: Actor::Human,
            reason: reason.clone(),
            session_id: None,
        },
    );
    let event = crate::models::ticket_transition_event::TicketTransitionEvent {
        key,
        status: to.as_str(),
        from: from.as_str(),
        to: to.as_str(),
        reason: reason.as_deref(),
        actor: Actor::Human.as_str(),
    };
    bus.emit(EVENT_TICKET_STATUS, serde_json::to_value(event)?);
    Ok(json!({
        "key": key,
        "from": from.as_str(),
        "to": to.as_str(),
        "path": path.display().to_string(),
    }))
}

/// Puts the ticket in `phase` and, for a GitHub mirror, writes the matching state label
/// (`to-test`, `testing`); a label failure is left to the next label write-back.
fn apply_phase_http(store: &TicketStore, key: &str, phase: model::Phase) -> anyhow::Result<()> {
    let (mut ticket, body, extra) = store.get(key)?;
    ticket.autopilot.set_phase(phase, SystemClock.now_utc());
    store.save(&mut ticket, &body, &extra)?;
    if ticket.github.is_some() && phase != model::Phase::Dev {
        let port = aidp_integrations::github_issues::GhIssuePort::new();
        if let Err(error) =
            aidp_tickets::github_sync::label_ticket(store, &port, key, phase.as_str())
        {
            tracing::warn!(key, %error, "phase label left for the next write-back");
        }
    }
    Ok(())
}

/// Puts the `to-test` tickets of `tickets` in the order the test lane takes them (manual
/// promotion first, then the oldest entry), each in a slot a `to-test` ticket already held.
fn order_to_test_lane(tickets: &mut [model::Ticket]) {
    let slots: Vec<usize> = (0..tickets.len())
        .filter(|&i| tickets[i].autopilot.phase() == model::Phase::ToTest)
        .collect();
    let mut lane: Vec<model::Ticket> = slots.iter().map(|&i| tickets[i].clone()).collect();
    lane.sort_by_key(|t| {
        (
            u8::from(t.autopilot.manual_priority != Some(true)),
            t.autopilot.phase_at.unwrap_or(t.updated),
            t.key.clone(),
        )
    });
    for (slot, ticket) in slots.into_iter().zip(lane) {
        tickets[slot] = ticket;
    }
}

/// What a manual start found out: launched or still queued (`Ok`), or refused for a
/// reason nameable synchronously (`Refused`).
enum StartOutcome {
    Ok(Value),
    Refused { status: u16, message: String },
}

/// Whether a manual start has a `-> queued` transition to make; a ticket already in
/// `queued` is only re-armed, as the self-edge is illegal even with `force`.
fn manual_start_needs_transition(from: Status) -> bool {
    from != Status::Queued
}

/// A `to-test` ticket still in progress is promoted in its lane, never re-queued.
fn is_to_test_in_progress(ticket: &model::Ticket) -> bool {
    ticket.status == Status::InProgress && ticket.autopilot.phase() == model::Phase::ToTest
}

/// Routing fields a manual start may override; an absent field keeps the ticket's own
/// value. Overrides are persisted on the note once the start passes validation.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct StartOverrides {
    pub service: Option<String>,
    pub template: Option<String>,
    pub track: Option<Track>,
    pub flow: Option<String>,
}

impl StartOverrides {
    /// Reads the optional `service`, `template` and `track` of a start request body; blank
    /// values count as absent and an unparsable `track` is an error.
    pub fn from_body(body: &Value) -> anyhow::Result<Self> {
        let field = |name: &str| {
            body[name]
                .as_str()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        };
        Ok(Self {
            service: field(SERVICE_FIELD),
            template: field(TEMPLATE_FIELD),
            track: field(TRACK_FIELD)
                .map(|value| value.parse::<Track>())
                .transpose()?,
            flow: field(FLOW_FIELD),
        })
    }

    /// Applies the template and track overrides onto `ticket`.
    pub fn apply(&self, ticket: &mut Ticket) {
        if let Some(template) = &self.template {
            ticket.template = Some(template.clone());
            // A template alone means "pipeline only": the routed flow must not win over it.
            ticket.flow = None;
        }
        if let Some(flow) = &self.flow {
            ticket.flow = Some(flow.clone());
        }
        if let Some(track) = self.track {
            ticket.track = Some(track);
        }
    }

    /// Refuses a `service` override that is reserved or unknown to the vault.
    pub fn check_service(&self, store: &TicketStore) -> anyhow::Result<()> {
        match &self.service {
            Some(service) => TicketStore::check_service(store.vault_root(), service, false),
            None => Ok(()),
        }
    }
}

/// Manual dispatch of one ticket: `force` reaches [`model::guard`] and the forced tick without
/// bypassing one session per project; validated `overrides` persist only after the guard passes.
fn manual_start_ticket(
    app: &Arc<App>,
    store: &TicketStore,
    key: &str,
    force: bool,
    overrides: &StartOverrides,
) -> anyhow::Result<StartOutcome> {
    let (current, _, _) = store.get(key)?;
    let from = current.status;
    let transition = manual_start_needs_transition(from) && !is_to_test_in_progress(&current);
    if transition {
        model::guard(from, Status::Queued, Actor::Human, force)?;
    }
    if let Some(service) = &overrides.service {
        store.move_to_service(key, service)?;
    }
    let (mut ticket, body, extra) = store.get(key)?;
    overrides.apply(&mut ticket);
    if transition {
        ticket.status = Status::Queued;
    }
    ticket.autopilot.manual_priority = Some(true);
    store.save(&mut ticket, &body, &extra)?;
    if transition {
        ledger::append(
            store.vault_root(),
            &Event {
                ts: SystemClock.now_utc(),
                key: key.to_string(),
                from: Some(from),
                to: Status::Queued,
                actor: Actor::Human,
                reason: Some(MANUAL_START_REASON.to_string()),
                session_id: None,
            },
        );
    }
    let report = if force {
        app.dispatcher.tick_local_forced(MANUAL_START_REASON, key)
    } else {
        app.dispatcher.tick_local(MANUAL_START_REASON)
    };

    let launched_now = report["launched"]
        .as_array()
        .is_some_and(|a| a.iter().any(|v| v.as_str() == Some(key)));
    if launched_now {
        return Ok(StartOutcome::Ok(report));
    }

    if !force && report["mode"] == json!(MODE_PAUSED) {
        return Ok(StartOutcome::Refused {
            status: HTTP_LOCKED,
            message: DISPATCHER_PAUSED_MESSAGE.to_string(),
        });
    }

    if let Some(entry) = report[REASON_SKIPPED]
        .as_array()
        .and_then(|a| a.iter().find(|e| e["key"].as_str() == Some(key)))
    {
        let reason = entry["reason"]
            .as_str()
            .unwrap_or(REASON_SKIPPED)
            .to_string();
        if matches!(
            AdoptionReason::from_wire(&reason),
            Some(AdoptionReason::Adopted | AdoptionReason::AdoptedMerged)
        ) {
            return Ok(StartOutcome::Ok(report));
        }
        return Ok(StartOutcome::Refused {
            status: HTTP_CONFLICT,
            message: format!("start refused: {reason}"),
        });
    }

    Ok(StartOutcome::Ok(report))
}

fn route(
    app: &Arc<App>,
    m: &Method,
    path: &str,
    query: &str,
    body: Value,
    creds: &ReqAuth<'_>,
) -> anyhow::Result<Reply> {
    let p = path.trim_start_matches("/api").trim_matches('/');
    let segs: Vec<&str> = if p.is_empty() {
        vec![]
    } else {
        p.split('/').collect()
    };
    let q = parse_query(query);
    let qs = |k: &str| q.get(k).cloned().filter(|v| !v.is_empty());
    let qn = |k: &str, d: usize| q.get(k).and_then(|v| v.parse::<usize>().ok()).unwrap_or(d);
    if *m == Method::Options {
        return Ok(Reply::Empty(HTTP_NO_CONTENT));
    }
    match app.static_ui.lookup(m, path, creds.accept_encoding) {
        StaticLookup::File(file) => return Ok(Reply::Static(file)),
        StaticLookup::NotFound => return Ok(err(HTTP_NOT_FOUND, ROUTE_NOT_FOUND)),
        StaticLookup::NotUi => {}
    }

    if let Some(reply) = crate::catalog::handle(app.skill_root.clone(), m, path, &body) {
        return Ok(Reply::Json(reply.status, reply.body));
    }

    if let Some((status, reply)) =
        crate::doctor::http_routes::handle(app.doctor.as_ref(), app.flows.as_ref(), m, path)
    {
        return Ok(Reply::Json(status, reply));
    }
    if segs.first() == Some(&FLOWS_SEGMENT) {
        let Some(engine) = app.flows.as_ref() else {
            return Ok(err(HTTP_SERVICE_UNAVAILABLE, FLOWS_UNAVAILABLE_MESSAGE));
        };
        if let Some(reply) = crate::flows::http_routes::handle(engine, m, path, &q, &body) {
            return Ok(Reply::Json(reply.status, reply.body));
        }
    }

    if let Some((status, reply)) = crate::settings_routes::handle(app, m, path, &body) {
        return Ok(Reply::Json(status, reply));
    }
    if let Some((status, reply)) = crate::unity_routes::handle(app, m, path, &q, &body) {
        return Ok(Reply::Json(status, reply));
    }
    if let Some((status, reply)) = crate::manual_run::handle(app, m, path, &body) {
        return Ok(Reply::Json(status, reply));
    }
    if let Some((status, reply)) = crate::memory_routes::handle(app, m, path, &q, &body) {
        return Ok(Reply::Json(status, reply));
    }
    if let Some((status, reply)) = crate::art_assets::handle(&app.native_runs, m, path, &q) {
        return Ok(Reply::Json(status, reply));
    }
    if let Some(reply) = crate::run_files::handle(&app.native_runs, m, path) {
        return Ok(match reply {
            crate::run_files::RunFilesReply::Json(status, value) => Reply::Json(status, value),
            crate::run_files::RunFilesReply::File(file) => Reply::Static(file),
            crate::run_files::RunFilesReply::Error(status, message) => err(status, message),
        });
    }

    if let Some(reply) = crate::native_runs::http_routes::handle(&app.native_runs, m, path, &body) {
        return Ok(Reply::Json(reply.status, reply.body));
    }

    Ok(match (m.clone(), segs.as_slice()) {
        (
            Method::Post,
            [crate::models::constants::mcp_configuration::RUNTIME_SEGMENT, crate::models::constants::mcp_configuration::CONFIGURATIONS_SEGMENT],
        ) => {
            let request = serde_json::from_value::<
                crate::models::mcp_configuration_request::McpConfigurationRequest,
            >(body);
            match request {
                Ok(request) => match app.sessions.mcp_configurations().upsert(request.id.clone(), request.configuration) {
                    Ok(()) => Reply::Json(HTTP_OK, serde_json::to_value(crate::models::mcp_configuration_registered::McpConfigurationRegistered { id: request.id })?),
                    Err(_) => err(HTTP_BAD_REQUEST, crate::models::constants::mcp_configuration::INVALID_CONFIGURATION),
                },
                Err(_) => err(HTTP_BAD_REQUEST, crate::models::constants::mcp_configuration::INVALID_REQUEST),
            }
        }
        (Method::Get, ["gateway", "status"]) => Reply::Json(HTTP_OK, status_json(app)),

        (Method::Get, ["health"]) => Reply::Json(HTTP_OK, health_payload(app)),
        (Method::Post, ["health", "cleanup"]) => {
            let ids = app.sessions.purge_orphaned();
            let sweep = app.dispatcher.sweep_worktrees();
            Reply::Json(HTTP_OK, crate::health::cleanup_json(&ids, &sweep))
        }

        (Method::Post, ["events", "token"]) => Reply::Json(
            HTTP_OK,
            json!({ "token": mint_sse_token(app), "expiresIn": SSE_TOKEN_TTL.as_secs() }),
        ),
        (Method::Get, ["events"]) => {
            Reply::Stream(Box::new(event_stream(app, qn(QUERY_SINCE, 0) as u64)))
        }
        (Method::Get, ["events", "recent"]) => {
            Reply::Json(HTTP_OK, json!(app.bus.recent(qn(QUERY_LIMIT, 100), 0)))
        }

        (Method::Get, ["inbox"]) => {
            let sessions = app.sessions.search(None, None, None, 500);
            let runs = app.automations.list_runs(None, 200);
            let attention = app.pr_attention.list(true);
            let mut entries =
                crate::inbox::build_inbox(&sessions, &runs, &attention, &crate::time::now_iso());
            if let Some(engine) = app.flows.as_ref() {
                entries.extend(engine.inbox_entries());
            }
            entries.truncate(qn(QUERY_LIMIT, 50));
            Reply::Json(HTTP_OK, json!(entries))
        }

        (Method::Get, ["sessions"]) => Reply::Json(
            HTTP_OK,
            json!(app.sessions.search(
                qs(QUERY_STATUS).as_deref().and_then(SessionStatus::parse),
                qs(QUERY_PROJECT_ID).as_deref(),
                qs(QUERY_TEXT).as_deref(),
                qn(QUERY_LIMIT, 200)
            )),
        ),
        (Method::Post, ["sessions"]) => {
            let mut r: StartRequest =
                serde_json::from_value(body).map_err(|e| anyhow!("invalid body: {e}"))?;
            if r.origin.is_none() {
                r.origin = Some(Origin::new(ORIGIN_API));
            }
            r.auto_approve_gates = sanitize_auto_approve_gates(r.auto_approve_gates);
            match app.launcher.launch(r, false) {
                Ok(launched) => launched_reply(launched),
                Err(e) => api_err(e),
            }
        }
        (Method::Get, ["sessions", id]) => match app.sessions.get(id) {
            Some(s) => Reply::Json(HTTP_OK, json!(s)),
            None => err(HTTP_NOT_FOUND, SESSION_NOT_FOUND),
        },
        (Method::Get, ["sessions", id, "events"]) => {
            if app.sessions.get(id).is_none() {
                err(HTTP_NOT_FOUND, SESSION_NOT_FOUND)
            } else {
                Reply::Json(HTTP_OK, json!(app.sessions.events(id)))
            }
        }
        (Method::Get, ["sessions", id, "stream"]) => {
            if app.sessions.get(id).is_none() {
                err(HTTP_NOT_FOUND, SESSION_NOT_FOUND)
            } else {
                Reply::Stream(Box::new(session_stream(app, id)))
            }
        }
        (Method::Post, ["sessions", id, "reply"]) => res_reply(app.sessions.reply(id, &body)),
        (Method::Post, ["sessions", id, "interrupt"]) => res_reply(app.sessions.interrupt(id)),
        (Method::Post, ["sessions", id, "cancel"]) => res_reply(app.sessions.cancel(id)),
        (Method::Post, ["sessions", id, "resume"]) => resume(app, id, &body),
        (Method::Patch, ["sessions", id]) => match app.sessions.patch(id, &body) {
            Some(s) => Reply::Json(HTTP_OK, json!(s)),
            None => err(HTTP_NOT_FOUND, SESSION_NOT_FOUND),
        },
        (Method::Get, ["sessions", id, "export.md"]) => match app.sessions.export_markdown(id) {
            Some(md) => Reply::Text(HTTP_OK, md, CONTENT_TYPE_MARKDOWN),
            None => err(HTTP_NOT_FOUND, SESSION_NOT_FOUND),
        },
        (Method::Get, ["sessions", id, "changes"]) => match app.sessions.get(id) {
            None => err(HTTP_NOT_FOUND, SESSION_NOT_FOUND),
            Some(rec) => {
                let cwd = std::path::Path::new(&rec.cwd);
                if let Some(f) = qs(QUERY_FILE) {
                    let base = rec.start_sha.as_deref().unwrap_or("");
                    match crate::changes::file_diff(cwd, base, &f) {
                        Some(diff) => Reply::Text(HTTP_OK, diff, CONTENT_TYPE_TEXT),
                        None => err(HTTP_NOT_FOUND, NO_DIFF_MESSAGE),
                    }
                } else {
                    let want_pr = match qs(QUERY_PR) {
                        Some(v) => v != "0" && !v.eq_ignore_ascii_case("false"),
                        None => true,
                    };
                    let body = crate::changes::build_response(
                        cwd,
                        &rec.id,
                        &rec.cwd,
                        rec.start_sha.as_deref(),
                        want_pr,
                    );
                    Reply::Json(HTTP_OK, body)
                }
            }
        },
        (Method::Delete, ["sessions", id]) => {
            if app.sessions.delete(id) {
                ok()
            } else {
                err(HTTP_NOT_FOUND, SESSION_NOT_FOUND)
            }
        }
        (Method::Get, ["sessions", id, "run"]) => match app.sessions.get(id) {
            None => err(HTTP_NOT_FOUND, SESSION_NOT_FOUND),
            Some(rec) => match crate::runs::for_session_with_run_id(
                &rec.id,
                &rec.cwd,
                &rec.started_at,
                rec.workflow_run_id.as_deref(),
            ) {
                Some(payload) => Reply::Json(HTTP_OK, payload),
                None => err(HTTP_NOT_FOUND, NO_RUN_LINKED_MESSAGE),
            },
        },

        (Method::Get, ["automations"]) => Reply::Json(HTTP_OK, json!(app.automations.list())),
        (Method::Post, ["automations"]) => match app.automations.upsert(body) {
            Ok(a) => Reply::Json(HTTP_OK, json!(a)),
            Err(e) => api_err(e),
        },
        (Method::Put, ["automations", id]) => match app.automations.get(id) {
            None => err(HTTP_NOT_FOUND, AUTOMATION_NOT_FOUND),
            Some(cur) => {
                let mut merged = serde_json::to_value(&cur)?;
                if let (Some(m), Some(b)) = (merged.as_object_mut(), body.as_object()) {
                    for (k, v) in b {
                        m.insert(k.clone(), v.clone());
                    }
                }
                merged["id"] = json!(cur.id);
                match app.automations.upsert(merged) {
                    Ok(a) => Reply::Json(HTTP_OK, json!(a)),
                    Err(e) => api_err(e),
                }
            }
        },
        (Method::Delete, ["automations", id]) => {
            if app.automations.delete(id) {
                ok()
            } else {
                err(HTTP_NOT_FOUND, AUTOMATION_NOT_FOUND)
            }
        }
        (Method::Post, ["automations", id, "run"]) => match app.automations.get(id) {
            None => err(HTTP_NOT_FOUND, AUTOMATION_NOT_FOUND),
            Some(_) => {
                match app
                    .automations
                    .run_automation(id, ORIGIN_UI, json!({ "payload": body }))
                {
                    Ok(r) => Reply::Json(HTTP_OK, json!(r)),
                    Err(e) => api_err(e),
                }
            }
        },
        (Method::Post, ["automations", id, "test"]) => {
            match app.automations.test_automation(id, body) {
                Ok(v) => Reply::Json(HTTP_OK, v),
                Err(e) => err(HTTP_NOT_FOUND, e),
            }
        }
        (Method::Get, ["automations", id, "runs"]) => {
            Reply::Json(HTTP_OK, json!(app.automations.list_runs(Some(id), 50)))
        }
        (Method::Get, ["automation-runs"]) => Reply::Json(
            HTTP_OK,
            json!(app.automations.list_runs(None, qn(QUERY_LIMIT, 50))),
        ),

        (Method::Post, ["hooks", id]) => {
            if app.hook_token.is_empty() {
                return Ok(err(SERVICE_UNAVAILABLE, WEBHOOKS_DISABLED_MESSAGE));
            }
            let authed = if creds.hub_sig.is_empty() {
                ct_eq(&creds.bearer(), &app.hook_token)
            } else {
                let expected = format!(
                    "{HMAC_PREFIX}{}",
                    hmac_sha256_hex(app.hook_token.as_bytes(), creds.raw_body)
                );
                ct_eq(creds.hub_sig, &expected)
            };
            if !authed {
                return Ok(err(HTTP_UNAUTHORIZED, UNAUTHORIZED_MESSAGE));
            }
            if let Some(engine) = app.flows.as_ref() {
                crate::flows::recovery::wake_from_webhook(engine, &body);
            }
            for (repo, number) in crate::prs::pr_targets_from_webhook(creds.github_event, &body) {
                app.pr_status.invalidate(&app.bus, &repo, number);
            }
            if crate::squad_prs::is_inbox_event(creds.github_event) {
                app.squad_prs.invalidate(&app.bus);
                spawn_attention_pass(app);
            }
            if let Some((repo, sha)) =
                crate::prs::status_target_from_webhook(creds.github_event, &body)
            {
                app.pr_status.invalidate_by_sha(&app.bus, &repo, &sha);
            }
            match app.automations.handle_webhook(id, body) {
                Ok(r) => Reply::Json(
                    HTTP_OK,
                    json!({ "ok": true, "runId": r.id, "sessionId": r.session_id, "status": r.status }),
                ),
                Err(e) => api_err(e),
            }
        }

        (Method::Get, ["projects"]) => Reply::Json(HTTP_OK, json!(app.projects.list())),
        (Method::Post, ["projects"]) => {
            let path = body["path"]
                .as_str()
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow!(PATH_REQUIRED_MESSAGE))?;
            Reply::Json(HTTP_OK, json!(app.projects.add(path)))
        }
        (Method::Post, ["projects", DISCOVER_SEGMENT]) => {
            let requested: Vec<PathBuf> = body[BODY_ROOTS]
                .as_array()
                .map(|roots| {
                    roots
                        .iter()
                        .filter_map(Value::as_str)
                        .map(PathBuf::from)
                        .collect()
                })
                .unwrap_or_default();
            let roots = if requested.is_empty() {
                app.projects.parent_roots()
            } else {
                requested
            };
            let vault =
                (!app.vault_root.as_os_str().is_empty()).then_some(app.vault_root.as_path());
            Reply::Json(HTTP_OK, json!(app.projects.discover(&roots, vault)))
        }
        (Method::Post, ["projects", "import"]) => {
            let list: Vec<Project> =
                serde_json::from_value(body["projects"].clone()).unwrap_or_default();
            Reply::Json(HTTP_OK, json!({ "imported": app.projects.import(list) }))
        }
        (Method::Get, ["projects", id]) => match app.projects.get(id) {
            Some(p) => Reply::Json(HTTP_OK, json!(p)),
            None => err(HTTP_NOT_FOUND, PROJECT_NOT_FOUND),
        },
        (Method::Get, ["projects", id, crate::routes::CLAUDE_SESSIONS_SEGMENT]) => {
            match app.projects.get(id) {
                Some(project) => list_claude_sessions(&project.path),
                None => err(HTTP_NOT_FOUND, PROJECT_NOT_FOUND),
            }
        }
        (Method::Put | Method::Patch, ["projects", id]) => {
            if let Some(value) = body.get(BODY_TEST_CHECKOUT) {
                let path = value.as_str();
                if !value.is_null() && path.is_none() {
                    return Ok(err(HTTP_BAD_REQUEST, TEST_CHECKOUT_NOT_STRING));
                }
                match app.projects.set_test_checkout(id, path) {
                    Ok(Some(project)) => Reply::Json(HTTP_OK, json!(project)),
                    Ok(None) => err(HTTP_NOT_FOUND, PROJECT_NOT_FOUND),
                    Err(message) => err(HTTP_BAD_REQUEST, message),
                }
            } else {
                match app.projects.get(id) {
                    Some(project) => Reply::Json(HTTP_OK, json!(project)),
                    None => err(HTTP_NOT_FOUND, PROJECT_NOT_FOUND),
                }
            }
        }
        (Method::Delete, ["projects", id]) => {
            if app.projects.remove(id) {
                ok()
            } else {
                err(HTTP_NOT_FOUND, PROJECT_NOT_FOUND)
            }
        }

        (Method::Get, ["presets"]) => Reply::Json(HTTP_OK, json!(app.presets.list())),
        (Method::Post, ["presets"]) => match app.presets.upsert(body) {
            Ok(p) => Reply::Json(HTTP_OK, json!(p)),
            Err(e) => api_err(e),
        },
        (Method::Delete, ["presets", id]) => {
            if app.presets.remove(id) {
                ok()
            } else {
                err(HTTP_NOT_FOUND, PRESET_NOT_FOUND)
            }
        }
        (Method::Post, ["presets", id, "start"]) => match app.presets.start(&app.launcher, id) {
            Ok(launched) => launched_reply(launched),
            Err(e) => api_err(e),
        },

        (Method::Get, ["usage"]) => {
            let from = qs(QUERY_FROM).map(|v| crate::usage::normalize_bound(&v));
            let to = qs(QUERY_TO).map(|v| crate::usage::normalize_bound(&v));
            let group_by = qs(QUERY_GROUP_BY).unwrap_or_else(|| DEFAULT_USAGE_GROUP.to_string());
            let all = app.sessions.list(None, None, 100_000);
            if crate::ticket_cost::is_contextual_group(&group_by) {
                let report = crate::cost_routes::usage_report(
                    &all,
                    from.as_deref(),
                    to.as_deref(),
                    &group_by,
                    &app.vault_root,
                );
                return Ok(Reply::Json(HTTP_OK, json!(report)));
            }
            match crate::usage::aggregate(&all, from.as_deref(), to.as_deref(), &group_by) {
                Ok(r) => Reply::Json(HTTP_OK, json!(r)),
                Err(e) => api_err(e),
            }
        }
        (Method::Get, ["pixellab", "balance"]) => {
            let reply = crate::pixellab_usage::balance_report(&app.vault_root);
            Reply::Json(HTTP_OK, reply)
        }
        (Method::Get, ["cost", "anomalies"]) => {
            let from = qs(QUERY_FROM).map(|v| crate::usage::normalize_bound(&v));
            let to = qs(QUERY_TO).map(|v| crate::usage::normalize_bound(&v));
            let all = app.sessions.list(None, None, 100_000);
            let report = crate::cost_routes::anomaly_report(&all, from.as_deref(), to.as_deref());
            Reply::Json(HTTP_OK, json!(report))
        }
        (Method::Get, ["tickets", crate::routes::TICKET_COSTS_SEGMENT]) => {
            let all = app.sessions.list(None, None, 100_000);
            let keys: Option<Vec<String>> = qs(QUERY_KEYS).map(|raw| {
                raw.split(',')
                    .map(str::trim)
                    .filter(|key| !key.is_empty())
                    .map(str::to_string)
                    .collect()
            });
            let report =
                crate::cost_routes::ticket_costs_report(&all, &app.vault_root, keys.as_deref());
            Reply::Json(HTTP_OK, json!(report))
        }
        (Method::Get, ["tickets", key, crate::routes::TICKET_COST_SEGMENT]) => {
            let all = app.sessions.list(None, None, 100_000);
            let rate_limit = app.sessions.rate_limit();
            match crate::cost_routes::ticket_cost_report(
                key,
                &all,
                &app.vault_root,
                rate_limit.as_ref(),
            ) {
                Some(report) => Reply::Json(HTTP_OK, json!(report)),
                None => err(HTTP_NOT_FOUND, TICKET_COST_NOT_FOUND),
            }
        }
        (
            Method::Get,
            ["tickets", key, crate::routes::TICKET_COST_SEGMENT, crate::issue_cost::DETAIL_SEGMENT],
        ) => {
            let all = app.sessions.list(None, None, 100_000);
            Reply::Json(
                HTTP_OK,
                json!(crate::issue_cost::issue_cost_report(
                    &app.vault_root,
                    key,
                    &all
                )),
            )
        }
        (Method::Get, ["usage", "rate-limit"]) => Reply::Json(
            HTTP_OK,
            json!(app.sessions.rate_limit_history(qn(QUERY_LIMIT, 200))),
        ),

        (Method::Get, ["tickets"]) => {
            let store = ticket_store_or_503!(app);
            let status = match qs(QUERY_STATUS) {
                Some(s) => s
                    .split(',')
                    .map(|x| parse_ticket_status(x.trim()))
                    .collect::<anyhow::Result<Vec<_>>>()?,
                None => Vec::new(),
            };
            let source = match qs(QUERY_SOURCE) {
                Some(s) => Some(parse_ticket_source(&s)?),
                None => None,
            };
            let filter = ListFilter {
                status,
                service: qs(QUERY_SERVICE),
                source,
                archived: false,
            };
            let mut tickets = store.list(&filter);
            order_to_test_lane(&mut tickets);
            tickets.truncate(qn(QUERY_LIMIT, 200));
            let views: Vec<Value> = tickets
                .iter()
                .map(crate::ticket_sync::ticket_view)
                .collect();
            let mut views = json!(views);
            crate::ticket_flow_summary::attach_list(app.flows.as_ref(), &mut views);
            Reply::Json(HTTP_OK, views)
        }
        (Method::Post, ["tickets", "sync"]) => {
            let store = ticket_store_or_503!(app);
            let request = crate::ticket_sync::SyncRequest {
                source: qs(QUERY_SOURCE),
                repo: qs("repo"),
                limit: qs("limit").and_then(|v| v.parse().ok()),
                dry_run: qs("dry_run").is_some_and(|v| v == "1" || v == "true"),
            };
            match crate::ticket_sync::handle(&store, &request) {
                Ok(report) => Reply::Json(HTTP_OK, report),
                Err(e)
                    if e.downcast_ref::<crate::ticket_sync::SyncRefused>()
                        .is_some() =>
                {
                    err(HTTP_BAD_REQUEST, e)
                }
                Err(e) => api_err(e),
            }
        }
        (Method::Post, ["tickets"]) => {
            let store = ticket_store_or_503!(app);
            match create_local_ticket(&store, &body) {
                Ok(v) => Reply::Json(HTTP_OK, v),
                Err(e) => api_err(e),
            }
        }
        (Method::Get, ["worktrees"]) => {
            let _store = ticket_store_or_503!(app);
            Reply::Json(HTTP_OK, cached_worktrees(app, qs(QUERY_PROJECT))?)
        }
        (Method::Post, ["worktrees", "ensure"]) => {
            app.route_caches.worktrees.clear();
            let store = ticket_store_or_503!(app);
            match worktree_key(body).and_then(|key| ensure_worktree(app, &store, &key)) {
                Ok(response) => Reply::Json(HTTP_OK, serde_json::to_value(&response)?),
                Err(error) => api_err(error),
            }
        }
        (Method::Post, ["worktrees", "remove"]) => {
            app.route_caches.worktrees.clear();
            let store = ticket_store_or_503!(app);
            match worktree_key(body).and_then(|key| remove_ticket_worktree(app, &store, &key)) {
                Ok(response) => Reply::Json(HTTP_OK, serde_json::to_value(&response)?),
                Err(error) => api_err(error),
            }
        }
        (Method::Post, ["worktrees", "prune"]) => {
            app.route_caches.worktrees.clear();
            let _store = ticket_store_or_503!(app);
            let sweep = app.dispatcher.sweep_worktrees();
            Reply::Json(HTTP_OK, serde_json::to_value(&sweep)?)
        }
        (Method::Get, ["prs", "status"]) => {
            let number = qs(QUERY_NUMBER).and_then(|raw| raw.parse::<u64>().ok());
            let (Some(repo), Some(number)) = (qs(QUERY_REPO), number) else {
                return Ok(err(HTTP_BAD_REQUEST, PR_PARAMS_MESSAGE));
            };
            let refresh = qs(QUERY_REFRESH).as_deref() == Some(QUERY_REFRESH_ON);
            pr_reply(crate::prs::handle_status(
                &app.pr_status,
                &repo,
                number,
                refresh,
            ))
        }
        (Method::Get, ["prs"]) => {
            let query = match SquadPrsQuery::from_lookup(qs) {
                Ok(query) => query,
                Err(message) => return Ok(err(HTTP_BAD_REQUEST, message)),
            };
            if app.squad_prs.is_stale() {
                spawn_inbox_refresh(app);
            }
            pr_reply(crate::squad_prs::handle_list(
                &app.squad_prs,
                &app.pr_marks,
                &query,
                app.flows.as_deref(),
            ))
        }
        (Method::Get, ["github", "asset"]) => {
            let Some(raw) = qs(QUERY_URL) else {
                return Ok(err(HTTP_BAD_REQUEST, ASSET_URL_MESSAGE));
            };
            let url = match aidp_integrations::github_asset::allowed_asset_url(&raw) {
                Ok(url) => url,
                Err(message) => return Ok(err(HTTP_BAD_REQUEST, message)),
            };
            let url = if aidp_integrations::github_asset::is_attachment(&url) {
                let repo = match aidp_integrations::github_asset::checked_repo(
                    qs(QUERY_REPO).as_deref(),
                ) {
                    Ok(repo) => repo.to_string(),
                    Err(message) => return Ok(err(HTTP_BAD_REQUEST, message)),
                };
                match aidp_integrations::github_asset::signed_attachment_url(
                    &url,
                    &repo,
                    &aidp_integrations::squad::OsGhRunner,
                ) {
                    Ok(signed) => signed,
                    Err(message) => return Ok(err(ApiStatus::Upstream.status(), message)),
                }
            } else {
                url
            };
            let token = if aidp_integrations::github_asset::needs_token(&url) {
                aidp_integrations::github_asset::gh_token().ok()
            } else {
                None
            };
            match aidp_integrations::github_asset::fetch_asset(&url, token.as_deref()) {
                Ok(asset) => Reply::Asset(asset),
                Err(message) => err(ApiStatus::Upstream.status(), message),
            }
        }
        (Method::Get, ["github", "rate-limit"]) => {
            let refresh = qs(QUERY_REFRESH).as_deref() == Some(QUERY_REFRESH_ON);
            match app.github_rate_limit.get(refresh) {
                Ok(limits) => Reply::Json(HTTP_OK, serde_json::to_value(limits)?),
                Err(message) => err(ApiStatus::Upstream.status(), message),
            }
        }
        (Method::Post, ["prs", "refresh"]) => pr_reply(crate::squad_prs::handle_refresh(
            &app.squad_prs,
            &app.pr_marks,
            &app.pr_attention,
            &app.bus,
            app.flows.as_deref(),
        )),
        (Method::Get, ["prs", "attention"]) => {
            let unread = qs(QUERY_UNREAD).as_deref() == Some(QUERY_UNREAD_ON);
            let items = app.pr_attention.list(unread);
            Reply::Json(
                HTTP_OK,
                serde_json::to_value(AttentionListResponse { items })?,
            )
        }
        (Method::Post, ["prs", "attention", "read"]) => {
            let request: AttentionReadRequest = match serde_json::from_value(body) {
                Ok(request) => request,
                Err(error) => return Ok(err(HTTP_BAD_REQUEST, format!("invalid body: {error}"))),
            };
            let has_fingerprints = request.fingerprints.as_ref().is_some_and(|f| !f.is_empty());
            if request.all != Some(true) && !has_fingerprints {
                return Ok(err(HTTP_BAD_REQUEST, ATTENTION_READ_MESSAGE));
            }
            let response = AttentionReadResponse {
                count: app.pr_attention.mark_read(&request),
            };
            let payload = serde_json::to_value(response)?;
            app.bus.emit(ATTENTION_READ_EVENT, payload.clone());
            Reply::Json(HTTP_OK, payload)
        }
        (Method::Post, ["prs", "marks"]) => {
            let request: PrMarkRequest = match serde_json::from_value(body) {
                Ok(request) => request,
                Err(error) => return Ok(err(HTTP_BAD_REQUEST, format!("invalid body: {error}"))),
            };
            pr_reply(crate::squad_prs::handle_mark(
                &app.squad_prs,
                &app.pr_marks,
                &app.bus,
                request,
            ))
        }
        (Method::Get, ["prs", "comments", "state"]) => {
            let number = qs(QUERY_NUMBER).and_then(|raw| raw.parse::<u64>().ok());
            let (Some(repo), Some(number)) = (qs(QUERY_REPO), number) else {
                return Ok(err(HTTP_BAD_REQUEST, PR_PARAMS_MESSAGE));
            };
            match app.pr_comment_state.read(&repo, number) {
                Ok(state) => Reply::Json(HTTP_OK, serde_json::to_value(state)?),
                Err(error) => err(HTTP_BAD_REQUEST, error),
            }
        }
        (Method::Post, ["prs", "comments", "state"]) => {
            let request: CommentStateRequest = match serde_json::from_value(body) {
                Ok(request) => request,
                Err(error) => return Ok(err(HTTP_BAD_REQUEST, format!("invalid body: {error}"))),
            };
            match app.pr_comment_state.set_point(
                &request.repo,
                request.number,
                &request.point_id,
                request.state,
            ) {
                Ok(state) => Reply::Json(HTTP_OK, serde_json::to_value(state)?),
                Err(error) => err(HTTP_BAD_REQUEST, error),
            }
        }
        (Method::Post, ["prs", "fix"]) => {
            let request: crate::models::pr_fix_request::PrFixRequest =
                match serde_json::from_value(body) {
                    Ok(request) => request,
                    Err(error) => {
                        return Ok(err(HTTP_BAD_REQUEST, format!("invalid body: {error}")))
                    }
                };
            pr_reply(crate::pr_fix::handle_fix(app, request))
        }
        (Method::Get, ["prs", "detail"]) => {
            pr_detail_call(app, pr_query(&q), |c, v| prd::handle_detail(c, &v))
        }
        (Method::Get, ["prs", "files"]) => {
            pr_detail_call(app, pr_query(&q), |c, v| prd::handle_files(c, &v))
        }
        (Method::Get, ["prs", "file-diff"]) => {
            pr_detail_call(app, pr_query(&q), |c, v| prd::handle_file_diff(c, &v))
        }
        (Method::Post, ["prs", "comments"]) => {
            pr_detail_call(app, pr_body(body), prd::handle_comment_create)
        }
        (Method::Patch, ["prs", "comments", id]) => {
            let input = pr_comment_id(id).and_then(|id| pr_body(body).map(|b| (id, b)));
            pr_detail_call(app, input, |c, (id, b)| {
                prd::handle_comment_edit(c, CommentKind::Conversation, id, b)
            })
        }
        (Method::Delete, ["prs", "comments", id]) => {
            let input = pr_comment_id(id).and_then(|id| pr_query(&q).map(|v| (id, v)));
            pr_detail_call(app, input, |c, (id, v)| {
                prd::handle_comment_delete(c, CommentKind::Conversation, id, &v)
            })
        }
        (Method::Post, ["prs", "review-comments"]) => {
            pr_detail_call(app, pr_body(body), prd::handle_review_comment_create)
        }
        (Method::Patch, ["prs", "review-comments", id]) => {
            let input = pr_comment_id(id).and_then(|id| pr_body(body).map(|b| (id, b)));
            pr_detail_call(app, input, |c, (id, b)| {
                prd::handle_comment_edit(c, CommentKind::Review, id, b)
            })
        }
        (Method::Delete, ["prs", "review-comments", id]) => {
            let input = pr_comment_id(id).and_then(|id| pr_query(&q).map(|v| (id, v)));
            pr_detail_call(app, input, |c, (id, v)| {
                prd::handle_comment_delete(c, CommentKind::Review, id, &v)
            })
        }
        (Method::Get, ["prs", "drafts"]) => {
            pr_detail_call(app, pr_query(&q), |c, v| prd::handle_drafts_get(c, &v))
        }
        (Method::Put, ["prs", "drafts"]) => {
            pr_detail_call(app, pr_body(body), prd::handle_drafts_put)
        }
        (Method::Post, ["prs", "review"]) => {
            pr_detail_call(app, pr_body(body), prd::handle_review_submit)
        }
        (Method::Post, ["prs", "viewed"]) => pr_detail_call(app, pr_body(body), prd::handle_viewed),
        (Method::Post, ["prs", "ready"]) => pr_action(app, body, crate::pr_actions::handle_ready),
        (Method::Post, ["prs", "update-branch"]) => {
            pr_action(app, body, crate::pr_actions::handle_update_branch)
        }
        (Method::Post, ["prs", "merge"]) => pr_action(app, body, crate::pr_actions::handle_merge),
        (Method::Post, ["prs", "threads", "reply"]) => {
            pr_action(app, body, crate::pr_actions::handle_thread_reply)
        }
        (Method::Post, ["prs", "threads", "resolve"]) => {
            pr_action(app, body, crate::pr_actions::handle_threads_resolve)
        }
        (Method::Get, ["tickets", key, "pr"]) => {
            let store = ticket_store_or_503!(app);
            let ticket = match store.get(key) {
                Ok((ticket, _body, _extra)) => ticket,
                Err(e) => return Ok(err(HTTP_NOT_FOUND, e)),
            };
            let Some(project) = app.projects.get(&ticket.service) else {
                return Ok(err(
                    HTTP_NOT_FOUND,
                    format!("{NO_PROJECT_MESSAGE_PREFIX}{}", ticket.service),
                ));
            };
            let project_dir = PathBuf::from(project.path);
            let refresh = qs(QUERY_REFRESH).as_deref() == Some(QUERY_REFRESH_ON);
            pr_reply(crate::prs::handle_ticket_pr_cached(
                &app.pr_status,
                &ticket,
                key,
                &project_dir,
                refresh,
            ))
        }
        (Method::Get, ["tickets", key, "worktree"]) => {
            let store = ticket_store_or_503!(app);
            match store.get(key) {
                Ok((ticket, _body, _extra)) => {
                    Reply::Json(HTTP_OK, json!(ticket_worktree(app, &store, &ticket)))
                }
                Err(e) => err(HTTP_NOT_FOUND, e),
            }
        }
        (Method::Post, ["tickets", key, crate::routes::TICKET_FLOW_SEGMENT]) => {
            let Some(engine) = app.flows.as_ref() else {
                return Ok(err(HTTP_SERVICE_UNAVAILABLE, FLOWS_UNAVAILABLE_MESSAGE));
            };
            let store = ticket_store_or_503!(app);
            let ticket = match store.get(key) {
                Ok((ticket, _body, _extra)) => serde_json::to_value(&ticket)?,
                Err(e) => return Ok(err(HTTP_NOT_FOUND, e)),
            };
            let request =
                serde_json::from_value::<TicketFlowRequest>(body.clone()).unwrap_or_default();
            let reply = crate::flows::start_ticket_flow(engine, ticket, request);
            Reply::Json(reply.status, reply.body)
        }
        (Method::Get, ["tickets", key, crate::routes::TICKET_FLOW_SEGMENT]) => {
            let Some(engine) = app.flows.as_ref() else {
                return Ok(err(HTTP_SERVICE_UNAVAILABLE, FLOWS_UNAVAILABLE_MESSAGE));
            };
            let reply = crate::flows::read_ticket_flow(engine, key);
            Reply::Json(reply.status, reply.body)
        }
        (Method::Get, ["tickets", key]) => {
            let store = ticket_store_or_503!(app);
            match store.get(key) {
                Ok((ticket, ticket_body, _extra)) => {
                    let mut ticket = crate::ticket_sync::ticket_view(&ticket);
                    crate::ticket_flow_summary::attach_one(app.flows.as_ref(), &mut ticket);
                    Reply::Json(HTTP_OK, json!({ "ticket": ticket, "body": ticket_body }))
                }
                Err(e) => err(HTTP_NOT_FOUND, e),
            }
        }
        (Method::Post, ["tickets", key, "start"]) => {
            let store = ticket_store_or_503!(app);
            let force = body["force"].as_bool().unwrap_or(false);
            let overrides = match StartOverrides::from_body(&body)
                .and_then(|parsed| parsed.check_service(&store).map(|()| parsed))
                .and_then(|parsed| match &parsed.flow {
                    Some(flow) => {
                        crate::manual_run::check_flow(app.sessions.configured_skill_root(), flow)
                            .map(|()| parsed)
                    }
                    None => Ok(parsed),
                }) {
                Ok(parsed) => parsed,
                Err(e) => return Ok(api_err(&e)),
            };
            match manual_start_ticket(app, &store, key, force, &overrides) {
                Ok(StartOutcome::Ok(mut v)) => {
                    crate::manual_run::attach_launch(&store, key, &mut v);
                    Reply::Json(HTTP_OK, v)
                }
                Ok(StartOutcome::Refused { status, message }) => err(status, message),
                Err(e) => err(HTTP_CONFLICT, e),
            }
        }
        (Method::Post, ["tickets", key, "adopt"]) => {
            let store = ticket_store_or_503!(app);
            let Some(reference) = serde_json::from_value::<AdoptRequest>(body.clone())
                .ok()
                .and_then(|request| request.pr.gh_reference())
            else {
                return Ok(err(HTTP_BAD_REQUEST, PR_REQUIRED_MESSAGE));
            };
            match app.dispatcher.adopt_ticket(&store, key, &reference) {
                Ok(response) => Reply::Json(HTTP_OK, serde_json::to_value(response)?),
                Err(e) => api_err(&e),
            }
        }
        (Method::Post, ["tickets", key, "cancel"]) => {
            let store = ticket_store_or_503!(app);
            let reason = body["reason"]
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| CANCELLED_BY_USER.to_string());
            match transition_ticket_http(
                &store,
                &app.bus,
                key,
                Status::Cancelled,
                Some(reason),
                true,
            ) {
                Ok(v) => Reply::Json(HTTP_OK, v),
                Err(e) => err(HTTP_CONFLICT, e),
            }
        }
        (Method::Post, ["tickets", key, "route"]) => {
            let store = ticket_store_or_503!(app);
            let Some(service) = body["service"].as_str() else {
                return Ok(err(HTTP_BAD_REQUEST, SERVICE_REQUIRED_MESSAGE));
            };
            let template = body["template"].as_str();
            match route_ticket_http(&store, key, service, template) {
                Ok(v) => Reply::Json(HTTP_OK, v),
                Err(e) => api_err(e),
            }
        }
        (Method::Post, ["tickets", key, "status"]) => {
            let store = ticket_store_or_503!(app);
            let Some(status_str) = body["status"].as_str() else {
                return Ok(err(HTTP_BAD_REQUEST, STATUS_REQUIRED_MESSAGE));
            };
            let to = match parse_ticket_status(status_str) {
                Ok(s) => s,
                Err(e) => return Ok(api_err(e)),
            };
            let reason = body["reason"].as_str().map(str::to_string);
            let force = body["force"].as_bool().unwrap_or(false);
            let phase = match body["phase"].as_str() {
                Some(text) => match model::Phase::parse(text) {
                    Some(phase) => Some(phase),
                    None => {
                        return Ok(err(
                            HTTP_BAD_REQUEST,
                            "phase must be dev, to-test or testing",
                        ))
                    }
                },
                None => None,
            };
            let same_status = phase.is_some()
                && store
                    .get(key)
                    .is_ok_and(|(current, _, _)| current.status == to);
            let moved = if same_status {
                Ok(json!({ "key": key, "from": to.as_str(), "to": to.as_str() }))
            } else {
                transition_ticket_http(&store, &app.bus, key, to, reason, force)
            };
            match moved.and_then(|mut v| {
                if let Some(phase) = phase {
                    apply_phase_http(&store, key, phase)?;
                    v["phase"] = json!(phase.as_str());
                }
                Ok(v)
            }) {
                Ok(v) => Reply::Json(HTTP_OK, v),
                Err(e) => err(HTTP_CONFLICT, e),
            }
        }

        (Method::Get, ["dispatcher"]) => Reply::Json(HTTP_OK, app.dispatcher.state_json()),
        (Method::Get, ["dispatcher", "plan"]) => Reply::Json(HTTP_OK, app.dispatcher.plan_only()),
        (Method::Get, ["dispatcher", "budget"]) => {
            Reply::Json(HTTP_OK, app.dispatcher.budget_json())
        }
        (Method::Post, ["dispatcher", "pause"]) => {
            let request: crate::models::DispatcherPauseRequest = match serde_json::from_value(body)
            {
                Ok(request) => request,
                Err(error) => return Ok(err(HTTP_BAD_REQUEST, format!("invalid body: {error}"))),
            };
            let actor = request
                .actor
                .as_deref()
                .unwrap_or(crate::models::constants::dispatcher_pause::DEFAULT_PAUSE_ACTOR);
            dispatcher_pause_reply(app.dispatcher.set_paused(request.paused, actor))
        }
        (Method::Post, ["dispatcher", "resume"]) => {
            dispatcher_pause_reply(app.dispatcher.set_paused(
                false,
                crate::models::constants::dispatcher_pause::DEFAULT_PAUSE_ACTOR,
            ))
        }
        (Method::Post, ["dispatcher", "tick"]) => {
            Reply::Json(HTTP_OK, app.dispatcher.tick_local(ORIGIN_API))
        }

        (Method::Get, ["queue"]) => Reply::Json(
            HTTP_OK,
            json!(app.sessions.queue_list(qs(QUERY_PROJECT_ID).as_deref())),
        ),
        (Method::Post, ["queue", id, "promote"]) => match app.sessions.queue_promote(id) {
            Some(v) => Reply::Json(HTTP_OK, v),
            None => err(HTTP_NOT_FOUND, QUEUE_ENTRY_NOT_FOUND),
        },
        (Method::Post, ["queue", id, "cancel"]) => match app.sessions.queue_cancel(id) {
            Some(_) => ok(),
            None => err(HTTP_NOT_FOUND, QUEUE_ENTRY_NOT_FOUND),
        },

        _ => err(HTTP_NOT_FOUND, ROUTE_NOT_FOUND),
    })
}

#[cfg(test)]
#[path = "../tests/unit/http_tests.rs"]
mod http_tests;

#[cfg(test)]
#[path = "../tests/unit/http_adopt_tests.rs"]
mod http_adopt_tests;
