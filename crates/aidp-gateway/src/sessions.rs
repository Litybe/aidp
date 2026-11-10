//! Sessions: each pipeline execution is a headless `claude -p` process speaking stream-json.
//! Records persist across restarts; a session found `running` at boot is marked `orphaned`.

use crate::bus::Bus;
use crate::claude_provider::{
    claude_launch_tail, user_message_line, ClaudeCodeProvider, CLAUDE_PROGRAM,
};
use crate::claude_runner::ClaudeProcessRunner;
use crate::ids::new_id;
use crate::models::constants::engine::ENGINE_WORKFLOW;
use crate::models::constants::pr_workspace_holder::{HOLDER_SESSION_PREFIX, HOLDER_WORKFLOW_STAGE};
use crate::models::constants::session_profile::{ADD_DIR_FLAG, ALLOWED_TOOLS_FLAG};
pub use crate::models::enums::OriginKind;
use crate::models::enums::SessionProfile;
use crate::models::enums::WorkspaceKind;
use crate::models::enums::{
    MessageRole, PermissionMode, SessionEventKind, SessionKind, SessionStatus, StreamMessageType,
    TextBlockType,
};
use crate::models::Workspace;
use crate::models::{
    ApiRefusal, ContentBlock, LogSessionEvent, ProjectRef, ReplyContent, TextBlock,
    UserMessageBody, UserMessageEvent, UserStreamMessage, WorkspaceRequest,
};
use crate::observability::{FIELD_SESSION_ID, FIELD_TICKET_KEY};
use crate::paths::{skill_generation, skill_root};
use crate::projects::Projects;
use crate::queue::{Admission, QueuedLaunch, WriterLane};
use crate::store::{append_jsonl, ensure_dir, read_json, read_jsonl, write_json};
use crate::sync::lock;
use crate::time::{now_iso, now_ms};
use crate::worktrees;
use aidp_agents::config::CLAUDE_PROVIDER;
use aidp_agents::PermissionPolicy;
use aidp_agents::{
    resolve_provider, AgentError, AgentEvent, AgentHandle, AgentProvider, AgentSpec,
    ProviderOrigin, ProvidersConfig, ResolveRequest,
};
use aidp_core::models::enums::AgentRole;
use aidp_core::process_runner::ProcessRunner;
#[cfg(test)]
use aidp_core::process_runner::{ProcessSpec, SpawnedChild};
use anyhow::{anyhow, bail};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tracing::{error, info, warn};

const ENGINE_PROMPT: &str = "prompt";
const ENGINE_V4: &str = "v4";
const FIELD_KEY: &str = "key";
const FIELD_PROJECT: &str = "project";
const FIELD_PATH: &str = "path";
const FIELD_BRANCH: &str = "branch";
const FIELD_REUSED: &str = "reused";
const FEATURE_INPUT: &str = "feature";
const AUTO_TITLE_MAX_CHARS: usize = 60;
const ATTACHMENTS_FIELD: &str = "attachments";
const MISSING_RESUME_ID_MESSAGE: &str = "resumeFrom is required when external is true";
const NO_HOME_MESSAGE: &str = "cannot locate the home directory";
/// Bus event published once a ticket worktree is ready for a session.
pub const EVENT_WORKTREE_CREATED: &str = "worktree.created";
/// Environment entry naming the only branch an autonomous session may push to.
pub const ALLOWED_PUSH_BRANCH_ENV: &str = "AIDP_ALLOWED_PUSH_BRANCH";
/// Launch warning when a worktree was requested but no vault is configured.
pub const WARNING_WORKTREE_NO_VAULT: &str = "worktree-unavailable: no vault";
const PERMISSION_MODE_DEFAULT: &str = "default";
const PERMISSION_MODE_ACCEPT_EDITS: &str = "acceptEdits";
const PERMISSION_MODE_BYPASS: &str = "bypassPermissions";
const PERMISSION_MODE_PLAN: &str = "plan";
const PERMISSION_MODE_AUTO: &str = "auto";
const PERMISSION_MODE_DONT_ASK: &str = "dontAsk";
const FIELD_PERMISSION_MODE: &str = "permissionMode";
const EFFORT_LOW: &str = "low";
const EFFORT_MEDIUM: &str = "medium";
const EFFORT_HIGH: &str = "high";
const EFFORT_XHIGH: &str = "xhigh";
const EFFORT_MAX: &str = "max";
const MAX_CHAIN_DEPTH_ENV: &str = "AIDP_MAX_CHAIN_DEPTH";
const CLAUDE_BIN_ENV: &str = "AIDP_CLAUDE_BIN";
const ALLOW_API_KEY_ENV: &str = "AIDP_ALLOW_API_KEY";
const ALLOW_API_KEY_ENABLED: &str = "1";
const ANTHROPIC_API_KEY_ENV: &str = "ANTHROPIC_API_KEY";
const ANTHROPIC_AUTH_TOKEN_ENV: &str = "ANTHROPIC_AUTH_TOKEN";
const SKILL_ROOT_ENV: &str = "AIDP_SKILL_ROOT";
const AUTO_APPROVE_GATES_ENV: &str = "AIDP_AUTO_APPROVE_GATES";
const AGENT_EVENT_POLL_INTERVAL: Duration = Duration::from_millis(10);
/// Attempts to read a session record a concurrent writer is replacing at load time.
const RECORD_READ_ATTEMPTS: u32 = 50;
/// Pause between two attempts of [`RECORD_READ_ATTEMPTS`].
const RECORD_READ_RETRY_INTERVAL: Duration = Duration::from_millis(10);
const MSG_SYSTEM: &str = "system";
const MSG_ASSISTANT: &str = "assistant";
const MSG_USER: &str = "user";
const MSG_RESULT: &str = "result";
const MSG_RATE_LIMIT_EVENT: &str = "rate_limit_event";
const BLOCK_TEXT: &str = "text";
const BLOCK_THINKING: &str = "thinking";
const BLOCK_TOOL_USE: &str = "tool_use";
const BLOCK_TOOL_RESULT: &str = "tool_result";
const INIT_SUBTYPE: &str = "init";
const DEFAULT_SYSTEM_SUBTYPE: &str = "info";
const ASK_USER_QUESTION_TOOL: &str = "AskUserQuestion";
const WORKFLOW_TOOL: &str = "Workflow";
const DEFAULT_TOOL_NAME: &str = "tool";
const UNKNOWN_RATE_LIMIT_STATUS: &str = "unknown";
const FIVE_HOUR_WINDOW: &str = "five_hour";
const SEVEN_DAY_WINDOW: &str = "seven_day";
const EVENT_RATE_LIMIT: &str = "gateway.rate_limit";
const EVENT_WORKFLOW_PROGRESS: &str = "session.workflow_progress";
const EVENT_QUESTION: &str = "session.question";
const EVENT_TURN_END: &str = "session.turn_end";
const EVENT_CHANGES: &str = "session.changes";
/// Bus event emitted when a session ends.
pub const EVENT_SESSION_ENDED: &str = "session.ended";
const EVENT_DEQUEUED: &str = "session.dequeued";
const EVENT_STARTED: &str = "session.started";
const EVENT_UPDATED: &str = "session.updated";
const EVENT_ANSWERED: &str = "session.answered";
const EVENT_PURGED: &str = "sessions.purged";
const DEQUEUE_FAILED: &str = "failed";
const DEQUEUE_STARTED: &str = "started";
const CONTROL_REQUEST: &str = "control_request";
const INTERRUPT_SUBTYPE: &str = "interrupt";
const INTERRUPTED_LOG: &str = "[interrupted by user]\n";
const SYSTEM_STREAM: &str = "system";
const NO_RESULT_TEXT: &str = "(no result text)";
const REQUEUED_ERROR: &str = "re-queued instead of starting";

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Origin {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub r#ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

impl Origin {
    /// True when this origin's kind is exactly `kind`.
    pub fn is(&self, kind: OriginKind) -> bool {
        self.kind == kind.as_str()
    }

    pub fn new(kind: &str) -> Self {
        Origin {
            kind: kind.to_string(),
            r#ref: None,
            label: None,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "camelCase")]
pub struct TokenUsage {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_create: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RateLimitWindow {
    pub utilization: f64,
    pub resets_at: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RateLimitInfo {
    pub status: String,
    pub is_using_overage: bool,
    pub five_hour: Option<RateLimitWindow>,
    pub seven_day: Option<RateLimitWindow>,
    pub observed_at: i64,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct PendingQuestion {
    pub tool_use_id: String,
    pub questions: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub permission_request_id: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct SessionRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_config_id: Option<String>,
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_session_id: Option<String>,
    pub project_id: String,
    pub project_name: String,
    pub cwd: String,
    pub engine: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inputs: Option<BTreeMap<String, String>>,
    pub prompt: String,
    pub permission_mode: String,
    /// The permission mode claude reported in its init message; it can differ from `permission_mode`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effective_permission_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_budget_usd: Option<f64>,
    pub status: SessionStatus,
    pub started_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// Claude Code's own session id — what `claude --resume` needs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resumed_from: Option<String>,
    /// The CLI's cost *estimate* — informational on a subscription.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    #[serde(default)]
    pub turns: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_question: Option<PendingQuestion>,
    pub origin: Origin,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub automation_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub automation_run_id: Option<String>,
    /// Run id of the `aidp run start` envelope seen on the session stream; first one wins.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_run_id: Option<String>,
    #[serde(default)]
    pub event_count: u64,
    #[serde(default)]
    pub auto_close: bool,
    /// Cumulative tokens — what actually counts against a Max subscription.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<TokenUsage>,
    /// User-editable label shown in the viewer sidebar (Lot 1a).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub archived: bool,
    #[serde(default)]
    pub unread: bool,
    /// Git HEAD of the session cwd at start; the base `GET /api/sessions/:id/changes` diffs against.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_sha: Option<String>,
    /// Gateway session id whose vault-run artefacts were folded into this
    /// session's prompt (Lot 6a).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_from: Option<String>,
    /// Depth of this session in a `contextFrom` chain. `None` for an
    /// unchained launch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chain_depth: Option<u32>,
    /// Where the session runs: the project checkout or a ticket worktree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<Workspace>,
    /// Non-fatal notes about how the launch was resolved.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    /// The session was launched headless; a resume of it stays headless.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub headless: bool,
}

impl SessionRecord {
    /// `Free` for a prompt session started from the viewer, `None` otherwise.
    pub fn kind(&self) -> Option<SessionKind> {
        (self.engine == ENGINE_PROMPT && self.origin.is(OriginKind::Ui))
            .then_some(SessionKind::Free)
    }
}

/// A worktree launch refused before any session was created.
#[derive(Debug, thiserror::Error)]
#[error("{detail}")]
pub struct WorktreeRefused {
    /// Stable machine-read value, the slug of the underlying worktree error.
    pub slug: &'static str,
    pub detail: String,
}

impl aidp_core::models::enums::HasErrorKind for WorktreeRefused {
    fn kind(&self) -> aidp_core::models::enums::ErrorKind {
        aidp_core::models::enums::ErrorKind::Conflict
    }
}

/// First prompt line cut to [`AUTO_TITLE_MAX_CHARS`] characters, or `None` for a blank prompt.
fn auto_title(prompt: Option<&str>) -> Option<String> {
    let line = prompt?
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())?;
    Some(line.chars().take(AUTO_TITLE_MAX_CHARS).collect())
}

/// The title given on the request, else the one derived from its prompt.
fn resolve_title(req: &StartRequest) -> Option<String> {
    req.title
        .as_deref()
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(str::to_string)
        .or_else(|| auto_title(req.prompt.as_deref()))
}

/// Attachments of a reply body; empty when the field is missing.
fn parse_attachments(body: &Value) -> anyhow::Result<Vec<crate::models::Attachment>> {
    let values: Vec<Value> = match body.get(ATTACHMENTS_FIELD) {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Array(values)) => values.clone(),
        Some(_) => bail!("invalid {ATTACHMENTS_FIELD}: expected an array"),
    };
    crate::attachments::parse_attachments(&values)
        .map_err(|error| anyhow::Error::new(ApiRefusal::from(error)))
}

/// Message content: the bare text, or a text block followed by the image blocks.
fn reply_content(text: Option<&str>, image_blocks: &[crate::models::ImageBlock]) -> ReplyContent {
    if image_blocks.is_empty() {
        return ReplyContent::Text(text.unwrap_or_default().to_string());
    }
    let text_block = text.map(|text| {
        ContentBlock::Text(TextBlock {
            block_type: TextBlockType::Text,
            text: text.to_string(),
        })
    });
    let image_blocks = image_blocks.iter().cloned().map(ContentBlock::Image);
    ReplyContent::Blocks(text_block.into_iter().chain(image_blocks).collect())
}

/// Where a launch runs and what to report about that choice.
struct ResolvedWorkspace {
    workspace: Workspace,
    warnings: Vec<String>,
}

/// Gate names an HTTP caller of `POST /api/sessions` may self-approve via `auto_approve_gates`.
/// Enforced in `http.rs` only; trusted in-process callers may pass the `*` wildcard.
pub const SELF_APPROVABLE_GATES_FROM_HTTP: &[&str] = &["pr"];

#[derive(Serialize, Deserialize, Debug, Default, Clone)]
#[serde(rename_all = "camelCase")]
pub struct StartRequest {
    pub project_id: String,
    /// v7 | v4 | prompt (default: v7 when a template is given, else prompt)
    pub engine: Option<String>,
    pub template_id: Option<String>,
    pub target: Option<String>,
    pub inputs: Option<BTreeMap<String, String>>,
    /// Raw prompt (engine `prompt`), or extra instructions appended for v7/v4.
    pub prompt: Option<String>,
    pub permission_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strict_mcp_config: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_config_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_servers: Option<Vec<Value>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub denied_shell_commands: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub denied_mcp_tools: BTreeMap<String, Vec<String>>,
    pub model: Option<String>,
    pub effort: Option<String>,
    /// Agent provider id overriding every other provider choice for this launch.
    #[serde(default)]
    pub provider: Option<String>,
    pub max_budget_usd: Option<f64>,
    pub origin: Option<Origin>,
    pub automation_id: Option<String>,
    pub automation_run_id: Option<String>,
    /// Gateway session id to resume (`claude --resume` on its claude session id).
    pub resume_from: Option<String>,
    /// End the process after the first completed turn with no pending question. Default: origin != "ui".
    pub auto_close: Option<bool>,
    /// Dispatch priority of a queued launch (descending, ties broken by queuedAt).
    pub priority: Option<i32>,
    /// Gateway session id whose vault-run artefacts are folded into this session's prompt (Lot 6a).
    pub context_from: Option<String>,
    /// Gates an autonomous session may answer itself; exported as `AIDP_AUTO_APPROVE_GATES`.
    pub auto_approve_gates: Option<Vec<String>>,
    /// Path to a JSON inputs file, handed to the driver as `--inputs-file`.
    pub inputs_file: Option<String>,
    /// Ticket key whose worktree the session runs in. Ignored for `ui` launches.
    #[serde(default)]
    pub worktree: Option<String>,
    /// Where the session runs; defaults to the project checkout.
    #[serde(default)]
    pub workspace: Option<WorkspaceRequest>,
    /// Label of the session; defaults to the first line of the prompt.
    #[serde(default)]
    pub title: Option<String>,
    /// When true, `resume_from` is a Claude Code transcript id rather than a gateway session id.
    #[serde(default)]
    pub external: Option<bool>,
    /// Extra environment variables exported to the session's agent process.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Writers sharing this non-empty group may run together in one workspace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shared_writer_group: Option<String>,
    /// Extra `--allowedTools` rules appended to the launch tail.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_tools: Vec<String>,
    /// Extra directories the session may read and write, appended as `--add-dir` to the launch tail.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub add_dirs: Vec<PathBuf>,
    /// Never relay a permission prompt: `default` becomes `dontAsk`, unallowed calls are denied.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub headless: bool,
    /// Who launches the agent sessions of a native run. Set in process by a flow launch, never read from a request body.
    #[serde(skip)]
    pub agent_executor: crate::models::enums::AgentExecutor,
}

/// Runs once, after the session `rec` ended and its `session.ended` event was published.
pub type SessionEndHook = Box<dyn FnOnce(&SessionRecord) + Send>;

/// The user's providers config, or the default one when it is missing or invalid.
fn load_providers_config() -> ProvidersConfig {
    let Some(home) = aidp_core::vault::home_dir() else {
        return ProvidersConfig::default();
    };
    ProvidersConfig::load(&home).unwrap_or_else(|error| {
        warn!("providers config ignored: {error}");
        ProvidersConfig::default()
    })
}

struct Live {
    stdin: Mutex<Option<Box<dyn Write + Send>>>,
    /// `None` once the event thread saw the agent exit; cancel is then a no-op.
    handle: Mutex<Option<AgentHandle>>,
    events: Mutex<Vec<Value>>,
    last_turn_error: AtomicBool,
    /// `tool_use` ids of `Workflow` blocks, so the matching `tool_result` starts a tail.
    workflow_tool_uses: Mutex<HashSet<String>>,
    /// Workflow run ids already tailed, so a resume never spawns a second watcher.
    workflow_runs: Mutex<HashSet<String>>,
    /// Set at session end to retire every watcher thread of this session.
    workflow_stop: Arc<AtomicBool>,
    permission_requests: Mutex<
        std::collections::VecDeque<
            crate::models::hosted_permission_request::HostedPermissionRequest,
        >,
    >,
}

pub struct SessionManager {
    dir: PathBuf,
    /// The gateway state dir — the PARENT of `dir` (the `state_dir` argument
    /// of `load`). Where the rate-limit history and the queue file live.
    state_dir: PathBuf,
    index: Mutex<HashMap<String, SessionRecord>>,
    live: Mutex<HashMap<String, Arc<Live>>>,
    bus: Arc<Bus>,
    projects: Arc<Projects>,
    rate_limit: Mutex<Option<RateLimitInfo>>,
    queue: Arc<crate::queue::Queue>,
    /// Makes 'project already running?' and 'record the launch' atomic in `start`.
    admission: Mutex<()>,
    /// Workspace keys reserved by writers that are not sessions, with the stage name when known.
    held_workspaces: Mutex<HashMap<String, Option<String>>>,
    /// Shared-writer group of each live session, by session id.
    session_writer_groups: Mutex<HashMap<String, String>>,
    /// One-shot callbacks run when the session of that id ends.
    end_hooks: Mutex<HashMap<String, SessionEndHook>>,
    /// Set once `shutdown` starts: `start` refuses launches and no queued launch replays.
    shutting_down: AtomicBool,
    /// Source of `claude` child processes; tests inject a scripted runner.
    runner: Arc<dyn ProcessRunner>,
    /// The built-in provider; the only one whose stdin the manager drives directly.
    claude_provider: Arc<ClaudeCodeProvider>,
    /// Agent providers by id; `claude` is registered by default.
    providers: Mutex<BTreeMap<String, Arc<dyn AgentProvider>>>,
    /// Provider defaults and tier-to-model mapping; empty unless injected.
    providers_config: Mutex<ProvidersConfig>,
    runtime_environment: Mutex<Option<BTreeMap<String, String>>>,
    mcp_configurations: Arc<crate::models::mcp_configuration_registry::McpConfigurationRegistry>,
}

/// The checkout of a project, as a workspace.
fn project_workspace(path: &str) -> Workspace {
    Workspace {
        kind: WorkspaceKind::Project,
        path: PathBuf::from(path),
        branch: None,
        name: None,
    }
}

/// The test checkout a flow pipeline step works in: a step of a `workspace: project` flow (origin
/// `flow`, no ticket worktree, no explicit workspace) of a project that has one.
fn flow_test_checkout(
    req: &StartRequest,
    origin: &Origin,
    project: &crate::projects::Project,
) -> Option<String> {
    (origin.is(OriginKind::Flow) && req.worktree.is_none() && req.workspace.is_none())
        .then(|| project.test_checkout.clone())
        .flatten()
}

/// The admission key of a record: its workspace, else the checkout at its cwd.
fn workspace_key_of(record: &SessionRecord) -> String {
    match &record.workspace {
        Some(workspace) => workspace.key(),
        None => project_workspace(&record.cwd).key(),
    }
}

/// True when the queued launch asks for the read-only `plan` permission mode.
fn queued_read_only(entry: &QueuedLaunch) -> bool {
    serde_json::from_value::<StartRequest>(entry.request.clone())
        .map(|req| req.permission_mode.as_deref() == Some(PERMISSION_MODE_PLAN))
        .unwrap_or(false)
}

/// The shared-writer group the queued launch asked for, if any.
fn queued_writer_group(entry: &QueuedLaunch) -> Option<String> {
    serde_json::from_value::<StartRequest>(entry.request.clone())
        .ok()
        .and_then(|req| req.shared_writer_group)
}

/// The outcome of a launch attempt: either it started immediately, or the
/// project was already busy and it was parked in the queue.
pub enum Launch {
    Started(Box<SessionRecord>),
    Queued(Box<QueuedLaunch>),
}

impl Launch {
    pub fn id(&self) -> &str {
        match self {
            Launch::Started(rec) => &rec.id,
            Launch::Queued(q) => &q.id,
        }
    }

    pub fn status(&self) -> &str {
        match self {
            Launch::Started(rec) => rec.status.as_str(),
            Launch::Queued(q) => &q.status,
        }
    }

    pub fn record(&self) -> Option<&SessionRecord> {
        match self {
            Launch::Started(rec) => Some(rec.as_ref()),
            Launch::Queued(_) => None,
        }
    }

    pub fn to_json(&self) -> Value {
        match self {
            Launch::Started(rec) => json!(rec),
            Launch::Queued(q) => q.public(),
        }
    }
}

fn tail(s: &str, n: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    let start = chars.len().saturating_sub(n);
    chars[start..].iter().collect()
}

/// Case-insensitive `GET /api/sessions?q=` match on title, template, target and project name.
/// A blank needle matches everything.
fn matches_q(rec: &SessionRecord, needle: &str) -> bool {
    let n = needle.trim().to_lowercase();
    if n.is_empty() {
        return true;
    }
    [
        rec.title.as_deref(),
        rec.template_id.as_deref(),
        rec.target.as_deref(),
        Some(rec.project_name.as_str()),
    ]
    .into_iter()
    .flatten()
    .any(|f| f.to_lowercase().contains(&n))
}

/// Render a session transcript as standalone Markdown: title, metadata, one section per event.
/// Never panics on a missing key.
pub fn render_markdown(rec: &SessionRecord, events: &[Value]) -> String {
    let mut out = String::new();
    let heading = rec
        .title
        .as_deref()
        .filter(|t| !t.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| format!("Session {}", rec.id));
    out.push_str(&format!("# {heading}\n\n"));

    out.push_str("## Metadata\n\n");
    out.push_str(&format!("- **Project**: {}\n", rec.project_name));
    out.push_str(&format!("- **Engine**: {}\n", rec.engine));
    if let Some(t) = &rec.template_id {
        out.push_str(&format!("- **Template**: {t}\n"));
    }
    if let Some(t) = &rec.target {
        out.push_str(&format!("- **Target**: {t}\n"));
    }
    out.push_str(&format!("- **Status**: {}\n", rec.status.as_str()));
    out.push_str(&format!("- **Started**: {}\n", rec.started_at));
    if let Some(e) = &rec.ended_at {
        out.push_str(&format!("- **Ended**: {e}\n"));
    }
    out.push_str(&format!("- **Turns**: {}\n\n", rec.turns));

    out.push_str("## Transcript\n\n");
    for ev in events {
        let kind = ev.get("kind").and_then(Value::as_str).unwrap_or("");
        let parsed_kind = ev
            .get("kind")
            .cloned()
            .and_then(|k| serde_json::from_value::<SessionEventKind>(k).ok());
        match parsed_kind {
            Some(SessionEventKind::Text) => {
                let text = ev.get("text").and_then(Value::as_str).unwrap_or_default();
                out.push_str(text);
                out.push_str("\n\n");
            }
            Some(SessionEventKind::Thinking) => {
                let text = ev.get("text").and_then(Value::as_str).unwrap_or_default();
                out.push_str("*Thinking:* ");
                out.push_str(text);
                out.push_str("\n\n");
            }
            Some(SessionEventKind::UserMessage) => {
                let text = ev.get("text").and_then(Value::as_str).unwrap_or_default();
                out.push_str("**User:** ");
                out.push_str(text);
                out.push_str("\n\n");
            }
            Some(SessionEventKind::ToolUse) => {
                let name = ev
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or(DEFAULT_TOOL_NAME);
                let input = ev.get("input").cloned().unwrap_or(Value::Null);
                out.push_str(&format!(
                    "**Tool call: {name}**\n\n```json\n{}\n```\n\n",
                    serde_json::to_string_pretty(&input).unwrap_or_default()
                ));
            }
            Some(SessionEventKind::ToolResult) => {
                let content = ev
                    .get("content")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let is_error = ev.get("isError").and_then(Value::as_bool).unwrap_or(false);
                let label = if is_error {
                    "Tool result (error)"
                } else {
                    "Tool result"
                };
                out.push_str(&format!("**{label}:**\n\n```\n{content}\n```\n\n"));
            }
            Some(SessionEventKind::Raw) => {
                let raw = ev.get("raw").cloned().unwrap_or(Value::Null);
                out.push_str(&format!(
                    "**Raw event:**\n\n```json\n{}\n```\n\n",
                    serde_json::to_string_pretty(&raw).unwrap_or_default()
                ));
            }
            Some(SessionEventKind::System) => {
                let subtype = ev
                    .get("subtype")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                out.push_str(&format!(
                    "**System ({subtype}):**\n\n```json\n{}\n```\n\n",
                    serde_json::to_string_pretty(ev).unwrap_or_default()
                ));
            }
            Some(SessionEventKind::TurnEnd) => {
                let result = ev.get("result").and_then(Value::as_str).unwrap_or_default();
                out.push_str("**Turn end:** ");
                out.push_str(if result.is_empty() {
                    NO_RESULT_TEXT
                } else {
                    result
                });
                out.push_str("\n\n");
            }
            Some(SessionEventKind::WorkflowProgress) => {
                let run_id = ev.get("runId").and_then(Value::as_str).unwrap_or("");
                let agent = ev.get("agent").and_then(Value::as_str).unwrap_or("");
                let status = ev.get("status").and_then(Value::as_str).unwrap_or("");
                let phase = ev.get("phase").and_then(Value::as_str).unwrap_or("");
                out.push_str(&format!(
                    "**Workflow progress ({run_id}):** {agent} [{phase}] — {status}\n\n"
                ));
            }
            Some(SessionEventKind::Question) => {
                let questions = ev.get("questions").cloned().unwrap_or(Value::Null);
                out.push_str(&format!(
                    "**Question:**\n\n```json\n{}\n```\n\n",
                    serde_json::to_string_pretty(&questions).unwrap_or_default()
                ));
            }
            Some(SessionEventKind::UserReply) => {
                let answers = ev.get("answers").cloned().unwrap_or(Value::Null);
                out.push_str(&format!(
                    "**User reply:**\n\n```json\n{}\n```\n\n",
                    serde_json::to_string_pretty(&answers).unwrap_or_default()
                ));
            }
            Some(SessionEventKind::Log) => {
                let text = ev.get("text").and_then(Value::as_str).unwrap_or_default();
                out.push_str(&format!("```\n{text}\n```\n\n"));
            }
            Some(SessionEventKind::Error) => {
                let message = ev
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                out.push_str(&format!("**Error:** {message}\n\n"));
            }
            None => {
                out.push_str(&format!(
                    "**Unknown event ({kind}):**\n\n```json\n{}\n```\n\n",
                    serde_json::to_string_pretty(ev).unwrap_or_default()
                ));
            }
        }
    }
    out
}

/// Hard cap on `contextFrom` chain depth, overridable via
/// `AIDP_MAX_CHAIN_DEPTH` (mirrors `AIDP_MAX_UTILIZATION` in automations.rs).
pub const MAX_CHAIN_DEPTH: u32 = 3;

/// Error message `start()` refuses a new launch with once `shutdown()` has
/// begun; `http.rs` maps it to `503`.
pub const SHUTTING_DOWN_ERROR: &str = "gateway is shutting down";

/// Parse the raw `AIDP_MAX_CHAIN_DEPTH`, falling back to [`MAX_CHAIN_DEPTH`] when unset or invalid.
/// Deterministic seam for tests.
fn resolve_cap(raw: Option<String>) -> u32 {
    raw.and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(MAX_CHAIN_DEPTH)
}

/// Effective chain-depth cap: `AIDP_MAX_CHAIN_DEPTH` when parsable, else [`MAX_CHAIN_DEPTH`].
/// Every chain bound must read this, never the bare constant.
pub fn max_chain_depth() -> u32 {
    resolve_cap(std::env::var(MAX_CHAIN_DEPTH_ENV).ok())
}

/// The outcome of admitting (or refusing) a chained launch.
#[derive(Debug, PartialEq)]
pub enum ChainDecision {
    Admit { depth: u32 },
    Cycle(String),
    TooDeep { depth: u32, max: u32 },
}

/// Decide whether a chained launch may proceed; `ancestry` is the parent chain, parent first.
/// Refuses a cycle or a depth over the cap; callers `bail!`.
pub fn decide_chain(child_id: &str, ancestry: &[String], parent_depth: u32) -> ChainDecision {
    if ancestry.iter().any(|id| id == child_id) {
        return ChainDecision::Cycle(child_id.to_string());
    }
    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for id in ancestry {
        if !seen.insert(id.as_str()) {
            return ChainDecision::Cycle(id.clone());
        }
    }
    let depth = parent_depth + 1;
    let max = max_chain_depth();
    if depth > max {
        return ChainDecision::TooDeep { depth, max };
    }
    ChainDecision::Admit { depth }
}

/// `requested` wins when `Some`, else the parent's budget cap is inherited.
/// The cap is inherited, not decremented: `cost_usd` is only an estimate.
pub fn inherit_budget(parent: Option<f64>, requested: Option<f64>) -> Option<f64> {
    requested.or(parent)
}

fn build_prompt(req: &StartRequest, engine: &str, generation: &str) -> String {
    if engine == ENGINE_PROMPT {
        return req.prompt.clone().unwrap_or_default().trim().to_string();
    }
    let inputs: BTreeMap<String, String> = req
        .inputs
        .clone()
        .unwrap_or_default()
        .into_iter()
        .filter(|(_, v)| !v.trim().is_empty())
        .collect();
    let extra = req
        .prompt
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(|p| format!("\n\n{p}"))
        .unwrap_or_default();
    let template = req.template_id.clone().unwrap_or_default();

    if engine == ENGINE_V4 {
        let kv = inputs
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join(" ");
        let kv = if kv.is_empty() {
            String::new()
        } else {
            format!(" {kv}")
        };
        return format!("run pipeline v4 template={template}{kv}{extra}");
    }

    let mut p = format!("/run-pipeline-{generation} {template}");
    if let Some(t) = req
        .target
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
    {
        p.push(' ');
        p.push_str(t);
    }
    if !inputs.is_empty() {
        p.push_str(&format!(
            "\n\nPipeline inputs, already decided (pass them as --inputs JSON, do not ask again): {}",
            serde_json::to_string(&inputs).unwrap_or_default()
        ));
    }
    if let Some(path) = req
        .inputs_file
        .as_deref()
        .map(str::trim)
        .filter(|p| !p.is_empty())
    {
        p.push_str(&format!(
            "\n\nPipeline inputs, already decided, are in {path} (ASCII-escaped JSON): pass `--inputs-file {path}` to `aidp run start`, do not ask for them again."
        ));
    }
    p.push_str(
        "\nThe template and target above are final: do not ask which template or target to use.",
    );
    p + &extra
}

/// Appends one `--allowedTools` flag carrying `rules`; the tail is unchanged when `rules` is empty.
fn with_allowed_tools(mut tail: Vec<String>, rules: &[String]) -> Vec<String> {
    if !rules.is_empty() {
        tail.push(ALLOWED_TOOLS_FLAG.to_string());
        tail.extend(rules.iter().cloned());
    }
    tail
}

/// Appends one `--add-dir` flag carrying `directories`; the tail is unchanged when there are none.
fn with_add_dirs(mut tail: Vec<String>, directories: &[PathBuf]) -> Vec<String> {
    if !directories.is_empty() {
        tail.push(ADD_DIR_FLAG.to_string());
        tail.extend(
            directories
                .iter()
                .map(|directory| directory.to_string_lossy().to_string()),
        );
    }
    tail
}

/// The requested mode, else `default`; a headless launch runs `dontAsk` instead of `default`.
fn launch_permission_mode(requested: Option<&str>, headless: bool) -> String {
    let mode = requested
        .filter(|mode| !mode.is_empty())
        .unwrap_or(PERMISSION_MODE_DEFAULT);
    if headless && mode == PERMISSION_MODE_DEFAULT {
        PermissionMode::HEADLESS_DEFAULT.as_str().to_string()
    } else {
        mode.to_string()
    }
}

/// How the agent gets its tools in `mode`; only a non-headless `default` or `acceptEdits` asks.
/// A headless `dontAsk` refuses what is not allowed, on every provider.
fn permission_policy(mode: &str, headless: bool) -> PermissionPolicy {
    match mode {
        PERMISSION_MODE_PLAN => PermissionPolicy::Plan,
        PERMISSION_MODE_DONT_ASK if headless => PermissionPolicy::Headless,
        PERMISSION_MODE_BYPASS | PERMISSION_MODE_AUTO | PERMISSION_MODE_DONT_ASK => {
            PermissionPolicy::Bypass
        }
        _ if headless => PermissionPolicy::Headless,
        _ => PermissionPolicy::Default,
    }
}

/// Permission modes accepted by `claude --permission-mode`.
pub const PERMISSION_MODES: &[&str] = &[
    PERMISSION_MODE_DEFAULT,
    PERMISSION_MODE_ACCEPT_EDITS,
    PERMISSION_MODE_BYPASS,
    PERMISSION_MODE_PLAN,
    PERMISSION_MODE_AUTO,
    PERMISSION_MODE_DONT_ASK,
];

/// Effort levels accepted by `claude --effort`.
pub const EFFORTS: &[&str] = &[
    EFFORT_LOW,
    EFFORT_MEDIUM,
    EFFORT_HIGH,
    EFFORT_XHIGH,
    EFFORT_MAX,
];

/// `^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$`, without a regex crate.
fn is_safe_identifier(v: &str) -> bool {
    let mut chars = v.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphanumeric() => {}
        _ => return false,
    }
    v.len() <= 64 && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

/// Reject a `permissionMode` outside [`PERMISSION_MODES`].
pub fn validate_permission_mode(v: &str) -> anyhow::Result<()> {
    if PERMISSION_MODES.contains(&v) {
        return Ok(());
    }
    bail!(
        "permissionMode must be one of: {} (got {:?})",
        PERMISSION_MODES.join(", "),
        v
    )
}

/// Reject an `effort` outside [`EFFORTS`].
pub fn validate_effort(v: &str) -> anyhow::Result<()> {
    if EFFORTS.contains(&v) {
        return Ok(());
    }
    bail!(
        "effort must be one of: {} (got {:?})",
        EFFORTS.join(", "),
        v
    )
}

/// A character allow-list rather than an enum, so new model ids keep working.
pub fn validate_model(v: &str) -> anyhow::Result<()> {
    if is_safe_identifier(v) {
        return Ok(());
    }
    bail!("model may only contain letters, digits, '.', '_' and '-' (max 64 chars); got {v:?}")
}

/// Claude session ids are UUIDs: same allow-list as the model.
pub fn validate_claude_session_id(v: &str) -> anyhow::Result<()> {
    if is_safe_identifier(v) {
        return Ok(());
    }
    bail!("cannot resume: stored claude session id is not a valid identifier")
}

/// Build the `claude` command without a shell; `AIDP_CLAUDE_BIN` overrides the PATH lookup.
/// API-key env vars are stripped unless `AIDP_ALLOW_API_KEY=1`, to ride the Max subscription.
pub fn claude_command(
    args: &[String],
    cwd: &str,
    extra_env: &[(String, String)],
) -> anyhow::Result<Command> {
    let program = match std::env::var(CLAUDE_BIN_ENV)
        .ok()
        .filter(|v| !v.is_empty())
    {
        Some(bin) => {
            if !Path::new(&bin).is_absolute() || !Path::new(&bin).is_file() {
                bail!("{CLAUDE_BIN_ENV}={bin} is not an absolute path to a file");
            }
            PathBuf::from(bin)
        }
        None => aidp_integrations::resolve_program(CLAUDE_PROGRAM).ok_or_else(|| {
            anyhow!(
                "claude not found on PATH; set AIDP_CLAUDE_BIN to the claude executable (e.g. .../claude.cmd on Windows)"
            )
        })?,
    };
    let mut cmd = Command::new(program);
    cmd.args(args)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if std::env::var(ALLOW_API_KEY_ENV).ok().as_deref() != Some(ALLOW_API_KEY_ENABLED) {
        cmd.env_remove(ANTHROPIC_API_KEY_ENV);
        cmd.env_remove(ANTHROPIC_AUTH_TOKEN_ENV);
    }
    if let Some(root) = skill_root() {
        cmd.env(SKILL_ROOT_ENV, root);
    }
    for (k, v) in extra_env {
        cmd.env(k, v);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    Ok(cmd)
}

/// Kill the whole process tree: on Windows the `claude.cmd` shim spawns `node`, hence `taskkill /T`.
/// Used by [`crate::claude_runner::ClaudeSpawnedChild`].
pub(crate) fn kill_tree(child: &mut Child) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let _ = Command::new("taskkill")
            .args(["/T", "/F", "/PID", &child.id().to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(0x0800_0000)
            .status();
    }
    let _ = child.kill();
}

/// How long the stdout reader sleeps between two exit polls of its child.
const CHILD_EXIT_POLL_INTERVAL: Duration = Duration::from_millis(100);

/// Reads a listed session record, retrying while a writer's remove-then-rename leaves it
/// absent or partial, so a live record is never dropped from the loaded index.
fn read_record_with_retry(file: &Path) -> Option<SessionRecord> {
    for _ in 0..RECORD_READ_ATTEMPTS {
        if let Some(rec) = read_json::<Option<SessionRecord>>(file, None) {
            return Some(rec);
        }
        thread::sleep(RECORD_READ_RETRY_INTERVAL);
    }
    warn!("session record {} unreadable at load", file.display());
    None
}

/// Wait for `child` to exit without holding its mutex, so `cancel` and `shutdown` can kill it.
/// `None` when the status is unreadable or has no code.
pub(crate) fn wait_for_exit(child: &Mutex<Child>) -> Option<i32> {
    loop {
        let polled = lock(child).try_wait();
        match polled {
            Ok(Some(status)) => return status.code(),
            Ok(None) => thread::sleep(CHILD_EXIT_POLL_INTERVAL),
            Err(_) => return None,
        }
    }
}

impl SessionManager {
    /// Load with the production [`ClaudeProcessRunner`] — spawns the real
    /// `claude` CLI.
    pub fn load(state_dir: &Path, bus: Arc<Bus>, projects: Arc<Projects>) -> Arc<Self> {
        Self::load_with_runner(state_dir, bus, projects, Arc::new(ClaudeProcessRunner))
            .with_providers_config(load_providers_config())
    }

    /// Load with an injected [`ProcessRunner`], so a session runs with no `claude` on `PATH`.
    pub fn load_with_runner(
        state_dir: &Path,
        bus: Arc<Bus>,
        projects: Arc<Projects>,
        runner: Arc<dyn ProcessRunner>,
    ) -> Arc<Self> {
        let dir = state_dir.join("sessions");
        ensure_dir(&dir);
        let mut index = HashMap::new();
        if let Ok(rd) = std::fs::read_dir(&dir) {
            for entry in rd.flatten() {
                let p = entry.path();
                let name = p
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_default();
                if !name.ends_with(".json") {
                    continue;
                }
                if let Some(mut rec) = read_record_with_retry(&p) {
                    if rec.status == SessionStatus::Running {
                        rec.status = SessionStatus::Orphaned;
                        if rec.ended_at.is_none() {
                            rec.ended_at = Some(now_iso());
                        }
                        let _ = write_json(&p, &rec);
                    }
                    index.insert(rec.id.clone(), rec);
                }
            }
        }
        info!("{} persisted session(s) loaded", index.len());
        let rate_limit = crate::usage::latest_live_snapshot(state_dir, now_ms());
        info!(
            "{}",
            if rate_limit.is_some() {
                "restored a still-live 5h rate-limit window from history"
            } else {
                "no still-live 5h rate-limit window in history; utilization guard starts unarmed until the next rate_limit_event"
            }
        );
        let queue = Arc::new(crate::queue::Queue::load(state_dir, bus.clone()));
        let claude_provider = Arc::new(ClaudeCodeProvider::new(runner.clone()));
        let default_provider: Arc<dyn AgentProvider> = claude_provider.clone();
        let providers = BTreeMap::from([(default_provider.id().to_string(), default_provider)]);
        Arc::new(SessionManager {
            dir,
            state_dir: state_dir.to_path_buf(),
            index: Mutex::new(index),
            live: Mutex::new(HashMap::new()),
            bus,
            projects,
            rate_limit: Mutex::new(rate_limit),
            queue,
            admission: Mutex::new(()),
            held_workspaces: Mutex::new(HashMap::new()),
            session_writer_groups: Mutex::new(HashMap::new()),
            end_hooks: Mutex::new(HashMap::new()),
            shutting_down: AtomicBool::new(false),
            runner,
            claude_provider,
            providers: Mutex::new(providers),
            providers_config: Mutex::new(ProvidersConfig::default()),
            runtime_environment: Mutex::new(None),
            mcp_configurations: Arc::new(Default::default()),
        })
    }

    /// Gives child agents this instance's environment, overriding conflicting launch values.
    pub fn with_runtime_environment(
        self: Arc<Self>,
        environment: BTreeMap<String, String>,
    ) -> Arc<Self> {
        *lock(&self.runtime_environment) = Some(environment);
        self
    }

    /// Ephemeral MCP definitions shared by every launch path of this runtime.
    pub fn mcp_configurations(
        &self,
    ) -> &Arc<crate::models::mcp_configuration_registry::McpConfigurationRegistry> {
        &self.mcp_configurations
    }

    pub(crate) fn runtime_environment(&self) -> BTreeMap<String, String> {
        lock(&self.runtime_environment).clone().unwrap_or_default()
    }

    pub(crate) fn configured_skill_root(&self) -> Option<PathBuf> {
        lock(&self.runtime_environment)
            .as_ref()
            .and_then(|environment| environment.get(SKILL_ROOT_ENV).map(PathBuf::from))
            .or_else(skill_root)
    }

    fn configured_vault(&self) -> anyhow::Result<Option<aidp_core::vault::Vault>> {
        let environment = lock(&self.runtime_environment);
        let Some(environment) = environment.as_ref() else {
            return Ok(aidp_core::vault::resolve_global());
        };
        let Some(value) = environment.get(crate::models::constants::flow_execution::VAULT_ENV)
        else {
            return Ok(None);
        };
        if aidp_core::vault::is_disabled(value) || value.trim().is_empty() {
            return Ok(None);
        }
        let root = Path::new(value);
        if !root.is_absolute() {
            bail!(crate::models::constants::launch_policy::INVALID_SCOPED_VAULT);
        }
        Ok(aidp_core::vault::resolve(Some(root), root))
    }

    /// Registers `provider` under `id`, replacing any provider already there.
    pub fn with_provider(self: Arc<Self>, id: &str, provider: Arc<dyn AgentProvider>) -> Arc<Self> {
        lock(&self.providers).insert(id.to_string(), provider);
        self
    }

    /// Sets the provider defaults and the tier-to-model mapping used at launch.
    pub fn with_providers_config(self: Arc<Self>, config: ProvidersConfig) -> Arc<Self> {
        *lock(&self.providers_config) = config;
        self
    }

    /// The provider and concrete model a launch with model tier `tier` runs on.
    fn resolve_launch_provider(
        &self,
        tier: Option<&str>,
        provider_override: Option<&str>,
    ) -> anyhow::Result<(Arc<dyn AgentProvider>, Option<String>)> {
        let request = ResolveRequest {
            role: AgentRole::Executor.as_str().to_string(),
            tier: tier.map(str::to_string),
            launch_override: provider_override.map(str::to_string),
            ..ResolveRequest::default()
        };
        let resolved = resolve_provider(&request, &lock(&self.providers_config))?;
        let provider = lock(&self.providers)
            .get(&resolved.provider)
            .cloned()
            .ok_or(AgentError::UnknownProvider {
                name: resolved.provider,
                origin: ProviderOrigin::ConfigDefault,
            })?;
        Ok((provider, resolved.model))
    }

    /// Starts `spec` on `provider`. The built-in provider returns its stdin so this manager
    /// sends the prompts; any other provider delivers the prompt itself.
    fn launch_agent(
        &self,
        provider: &Arc<dyn AgentProvider>,
        spec: AgentSpec,
    ) -> anyhow::Result<(AgentHandle, Option<Box<dyn Write + Send>>)> {
        if std::ptr::addr_eq(Arc::as_ptr(provider), Arc::as_ptr(&self.claude_provider)) {
            let launch = self.claude_provider.start_with_stdin(spec)?;
            let stdin = launch
                .stdin
                .map(|stdin| Box::new(stdin) as Box<dyn Write + Send>);
            return Ok((launch.handle, stdin));
        }
        Ok((provider.start(spec)?, None))
    }

    /// Feeds agent events into the session until it exits; returns the exit code.
    fn pump_agent_events(self: &Arc<Self>, id: &str, live: &Arc<Live>) -> Option<i32> {
        loop {
            let received = match lock(&live.handle).as_ref() {
                Some(handle) => handle.events().try_recv(),
                None => return None,
            };
            match received {
                Ok(AgentEvent::Raw(Value::String(text))) => {
                    let event = LogSessionEvent {
                        kind: SessionEventKind::Log,
                        text,
                    };
                    self.push_event(id, live, serde_json::to_value(event).unwrap_or(Value::Null))
                }
                Ok(AgentEvent::Raw(message)) => self.handle_message(id, live, message),
                Ok(AgentEvent::Exited { code, .. }) => {
                    lock(&live.handle).take();
                    return code;
                }
                Ok(AgentEvent::PermissionRequest {
                    id: request_id,
                    tool,
                    input,
                }) => self.queue_permission_request(id, live, request_id, tool, input),
                Ok(_) => {}
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    thread::sleep(AGENT_EVENT_POLL_INTERVAL)
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    lock(&live.handle).take();
                    return None;
                }
            }
        }
    }

    /// Asks the live agent to stop; a no-op once it has exited.
    fn cancel_agent(live: &Live) -> anyhow::Result<()> {
        if let Some(handle) = lock(&live.handle).as_ref() {
            handle.cancel()?;
            handle.wait()?;
        }
        Ok(())
    }

    fn meta_file(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }
    fn events_file(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.events.jsonl"))
    }

    fn save(&self, rec: &SessionRecord) {
        if let Err(e) = write_json(&self.meta_file(&rec.id), rec) {
            error!("cannot save session {}: {e}", rec.id);
        }
    }

    /// Mutate a record under the index lock, then persist a snapshot of it.
    fn update<R>(&self, id: &str, f: impl FnOnce(&mut SessionRecord) -> R) -> Option<R> {
        let snapshot;
        let r;
        {
            let mut idx = lock(&self.index);
            let rec = idx.get_mut(id)?;
            r = f(rec);
            snapshot = rec.clone();
        }
        self.save(&snapshot);
        Some(r)
    }

    fn snapshot(&self, id: &str) -> Option<SessionRecord> {
        lock(&self.index).get(id).cloned()
    }

    fn push_event(&self, id: &str, live: &Live, mut ev: Value) {
        ev["ts"] = json!(now_ms());
        lock(&live.events).push(ev.clone());
        append_jsonl(&self.events_file(id), &ev);
        if let Some(r) = lock(&self.index).get_mut(id) {
            r.event_count += 1;
        }
    }

    fn summary(rec: &SessionRecord) -> Value {
        json!({
            "sessionId": rec.id,
            "projectId": rec.project_id,
            "projectName": rec.project_name,
            "engine": rec.engine,
            "templateId": rec.template_id,
            "target": rec.target,
            "status": rec.status,
            "origin": rec.origin,
            "automationId": rec.automation_id,
            "automationRunId": rec.automation_run_id,
            "title": rec.title,
            "kind": rec.kind(),
            "pinned": rec.pinned,
            "archived": rec.archived,
            "unread": rec.unread,
        })
    }

    pub fn rate_limit(&self) -> Option<RateLimitInfo> {
        lock(&self.rate_limit).clone()
    }

    fn on_rate_limit(&self, msg: &Value) {
        let Some(info) = msg.get("rate_limit_info") else {
            return;
        };
        let win = |k: &str| -> Option<RateLimitWindow> {
            let w = info.pointer(&format!("/unifiedWindows/{k}"))?;
            Some(RateLimitWindow {
                utilization: w["utilization"].as_f64().unwrap_or(0.0),
                resets_at: w["resetsAt"].as_i64().unwrap_or(0),
            })
        };
        let next = RateLimitInfo {
            status: info["status"]
                .as_str()
                .unwrap_or(UNKNOWN_RATE_LIMIT_STATUS)
                .to_string(),
            is_using_overage: info["isUsingOverage"].as_bool().unwrap_or(false),
            five_hour: win(FIVE_HOUR_WINDOW),
            seven_day: win(SEVEN_DAY_WINDOW),
            observed_at: now_ms(),
        };
        let changed = {
            let cur = lock(&self.rate_limit);
            match cur.as_ref() {
                None => true,
                Some(c) => {
                    c.status != next.status
                        || c.five_hour.as_ref().map(|w| w.utilization)
                            != next.five_hour.as_ref().map(|w| w.utilization)
                        || c.seven_day.as_ref().map(|w| w.utilization)
                            != next.seven_day.as_ref().map(|w| w.utilization)
                }
            }
        };
        if changed {
            self.bus.emit(
                EVENT_RATE_LIMIT,
                json!({
                    "status": next.status,
                    "fiveHour": next.five_hour.as_ref().map(|w| w.utilization),
                    "sevenDay": next.seven_day.as_ref().map(|w| w.utilization),
                    "isUsingOverage": next.is_using_overage,
                }),
            );
            crate::usage::append_rate_limit(&self.state_dir, &next);
        }
        *lock(&self.rate_limit) = Some(next);
    }

    /// Persisted rate-limit snapshot history, newest first (see `usage.rs`).
    pub fn rate_limit_history(&self, limit: usize) -> Vec<RateLimitInfo> {
        crate::usage::read_rate_limit_history(&self.state_dir, limit)
    }

    /// Tail a launched `Workflow` run's journal on its own thread, publishing bus and JSONL events.
    /// Stops when `live.workflow_stop` is set at session end.
    fn start_workflow_watcher(
        self: &Arc<Self>,
        id: &str,
        live: &Arc<Live>,
        launch: crate::workflow_progress::WorkflowLaunch,
    ) {
        let (mgr, live2, session_id, bus, stop) = (
            self.clone(),
            live.clone(),
            id.to_string(),
            self.bus.clone(),
            live.workflow_stop.clone(),
        );
        crate::workflow_progress::watch(launch, stop, move |mut payload| {
            if let Value::Object(map) = &mut payload {
                map.insert("sessionId".to_string(), json!(session_id));
            }
            mgr.push_event(
                &session_id,
                &live2,
                json!({
                    "kind": SessionEventKind::WorkflowProgress,
                    "runId": payload["runId"],
                    "phase": payload["phase"],
                    "label": payload["label"],
                    "agent": payload["agent"],
                    "status": payload["status"],
                }),
            );
            bus.emit(EVENT_WORKFLOW_PROGRESS, payload);
        });
    }

    /// Store the session's run id from the first `aidp run start` envelope; later ones are ignored.
    /// The capture is logged with the session id and the ticket key of its origin.
    fn capture_run_id(&self, id: &str, run_id: String) {
        let mut ticket_key: Option<String> = None;
        let captured = self
            .update(id, |record| {
                if record.workflow_run_id.is_some() {
                    return false;
                }
                record.workflow_run_id = Some(run_id.clone());
                ticket_key = record.origin.r#ref.clone();
                true
            })
            .unwrap_or(false);
        if captured {
            info!(
                session_id = %id,
                run_id = %run_id,
                ticket_key = ticket_key.as_deref().unwrap_or_default(),
                "session run id captured"
            );
        }
    }

    fn queue_permission_request(
        self: &Arc<Self>,
        id: &str,
        live: &Arc<Live>,
        request_id: String,
        tool: String,
        input: Value,
    ) {
        lock(&live.permission_requests).push_back(
            crate::models::hosted_permission_request::HostedPermissionRequest {
                id: request_id,
                tool,
                input,
            },
        );
        self.show_hosted_permission(id, live);
    }

    fn show_hosted_permission(&self, id: &str, live: &Arc<Live>) {
        use crate::models::constants::claude_permissions;
        let requests = lock(&live.permission_requests);
        let Some(request) = requests.front().cloned() else {
            return;
        };
        if self
            .snapshot(id)
            .is_some_and(|record| record.pending_question.is_some())
        {
            return;
        }
        let description = format!("{}\n{}", request.tool, request.input);
        let questions =
            serde_json::to_value(vec![crate::models::provider_question::ProviderQuestion {
                question: claude_permissions::QUESTION.to_string(),
                header: claude_permissions::HEADER.to_string(),
                options: vec![
                    crate::models::provider_question_option::ProviderQuestionOption {
                        label: claude_permissions::DENY.to_string(),
                        description: description.clone(),
                    },
                    crate::models::provider_question_option::ProviderQuestionOption {
                        label: claude_permissions::ALLOW.to_string(),
                        description,
                    },
                ],
            }])
            .unwrap_or(Value::Null);
        self.update(id, |record| {
            record.pending_question = Some(PendingQuestion {
                tool_use_id: request.id.clone(),
                questions: questions.clone(),
                permission_request_id: Some(request.id.clone()),
            })
        });
        self.push_event(id, live, json!({ "kind": SessionEventKind::Question, "toolUseId": request.id, "questions": questions }));
        if let Some(record) = self.snapshot(id) {
            self.bus.emit(EVENT_QUESTION, Self::summary(&record));
        }
    }

    fn handle_message(self: &Arc<Self>, id: &str, live: &Arc<Live>, msg: Value) {
        match msg["type"].as_str().unwrap_or("") {
            MSG_SYSTEM => {
                if msg["subtype"] == INIT_SUBTYPE {
                    let mode = msg[FIELD_PERMISSION_MODE].as_str().map(str::to_string);
                    self.update(id, |r| r.effective_permission_mode = mode);
                    if let Some(sid) = msg["session_id"].as_str() {
                        let model = msg["model"].as_str().map(|s| s.to_string());
                        self.update(id, |r| {
                            r.claude_session_id = Some(sid.to_string());
                            if r.model.is_none() {
                                r.model = model;
                            }
                        });
                    }
                }
                let mut event = json!({
                    "kind": SessionEventKind::System,
                    "subtype": msg["subtype"].as_str().unwrap_or(DEFAULT_SYSTEM_SUBTYPE),
                    "sessionId": msg["session_id"],
                    "model": msg["model"],
                    "tools": msg["tools"],
                });
                if let Some(mode) = msg.get(FIELD_PERMISSION_MODE) {
                    event[FIELD_PERMISSION_MODE] = mode.clone();
                }
                self.push_event(id, live, event);
            }
            MSG_ASSISTANT => {
                let blocks = msg
                    .pointer("/message/content")
                    .and_then(|c| c.as_array())
                    .cloned()
                    .unwrap_or_default();
                for b in blocks {
                    match b["type"].as_str().unwrap_or("") {
                        BLOCK_TEXT => {
                            if let Some(t) = b["text"].as_str().filter(|t| !t.is_empty()) {
                                let t400 = tail(t, 400);
                                if let Some(r) = lock(&self.index).get_mut(id) {
                                    r.last_text = Some(t400);
                                }
                                self.push_event(
                                    id,
                                    live,
                                    json!({ "kind": SessionEventKind::Text, "text": t }),
                                );
                            }
                        }
                        BLOCK_THINKING => {
                            if let Some(t) = b["thinking"].as_str().filter(|t| !t.is_empty()) {
                                self.push_event(
                                    id,
                                    live,
                                    json!({ "kind": SessionEventKind::Thinking, "text": t }),
                                );
                            }
                        }
                        BLOCK_TOOL_USE => {
                            let tool_id = b["id"].as_str().unwrap_or("").to_string();
                            if b["name"] == ASK_USER_QUESTION_TOOL {
                                let questions = b
                                    .pointer("/input/questions")
                                    .cloned()
                                    .unwrap_or_else(|| json!([]));
                                let q2 = questions.clone();
                                let tid = tool_id.clone();
                                self.update(id, |r| {
                                    r.pending_question = Some(PendingQuestion {
                                        tool_use_id: tid,
                                        questions: q2,
                                        permission_request_id: None,
                                    })
                                });
                                self.push_event(
                                    id,
                                    live,
                                    json!({ "kind": SessionEventKind::Question, "toolUseId": tool_id, "questions": questions }),
                                );
                                if let Some(rec) = self.snapshot(id) {
                                    let qs: Vec<Value> = questions
                                        .as_array()
                                        .map(|a| a.iter().map(|q| q["question"].clone()).collect())
                                        .unwrap_or_default();
                                    let mut d = Self::summary(&rec);
                                    d["questions"] = json!(qs);
                                    self.bus.emit(EVENT_QUESTION, d);
                                }
                            } else {
                                if b["name"] == WORKFLOW_TOOL {
                                    lock(&live.workflow_tool_uses).insert(tool_id.clone());
                                }
                                self.push_event(
                                    id,
                                    live,
                                    json!({ "kind": SessionEventKind::ToolUse, "toolUseId": tool_id, "name": b["name"], "input": b["input"] }),
                                );
                            }
                        }
                        _ => {}
                    }
                }
            }
            MSG_USER => {
                if let Some(blocks) = msg.pointer("/message/content").and_then(|c| c.as_array()) {
                    for b in blocks {
                        if b["type"] == BLOCK_TOOL_RESULT {
                            let content = match &b["content"] {
                                Value::String(s) => s.clone(),
                                Value::Null => String::new(),
                                other => other.to_string(),
                            };
                            self.push_event(
                                id,
                                live,
                                json!({
                                    "kind": SessionEventKind::ToolResult,
                                    "toolUseId": b["tool_use_id"],
                                    "content": content,
                                    "isError": b["is_error"].as_bool().unwrap_or(false),
                                }),
                            );
                            if let Some(run_id) =
                                crate::workflow_progress::parse_run_start_run_id(&content)
                            {
                                self.capture_run_id(id, run_id);
                            }

                            if let Some(tool_use_id) = b["tool_use_id"].as_str() {
                                let is_workflow_tool_use =
                                    lock(&live.workflow_tool_uses).contains(tool_use_id);
                                if is_workflow_tool_use {
                                    if let Some(launch) =
                                        crate::workflow_progress::parse_workflow_launch(&content)
                                    {
                                        let is_new_run = {
                                            let mut runs = lock(&live.workflow_runs);
                                            if runs.contains(&launch.run_id) {
                                                false
                                            } else {
                                                runs.insert(launch.run_id.clone());
                                                true
                                            }
                                        };
                                        if is_new_run {
                                            self.start_workflow_watcher(id, live, launch);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            MSG_RESULT => {
                let is_error = msg["is_error"].as_bool().unwrap_or(false);
                live.last_turn_error.store(is_error, Ordering::SeqCst);
                let result_text = msg["result"].as_str().unwrap_or("").to_string();
                let subtype = msg["subtype"].as_str().map(|s| s.to_string());
                let cost = msg["total_cost_usd"].as_f64();
                let usage = msg.get("usage").cloned();
                let auto_close = self
                    .update(id, |r| {
                        r.turns += 1;
                        if let Some(c) = cost {
                            r.cost_usd = Some(c);
                        }
                        if let Some(u) = &usage {
                            let acc = r.usage.get_or_insert_with(TokenUsage::default);
                            acc.input += u["input_tokens"].as_u64().unwrap_or(0);
                            acc.output += u["output_tokens"].as_u64().unwrap_or(0);
                            acc.cache_read += u["cache_read_input_tokens"].as_u64().unwrap_or(0);
                            acc.cache_create +=
                                u["cache_creation_input_tokens"].as_u64().unwrap_or(0);
                        }
                        if !result_text.is_empty() {
                            r.last_text = Some(tail(&result_text, 400));
                        } else if is_error {
                            if let Some(s) = &subtype {
                                r.last_text = Some(format!("[{s}]"));
                            }
                        }
                        r.auto_close && r.pending_question.is_none()
                    })
                    .unwrap_or(false);
                self.push_event(
                    id,
                    live,
                    json!({
                        "kind": SessionEventKind::TurnEnd,
                        "result": result_text,
                        "subtype": subtype,
                        "isError": is_error,
                        "costUsd": cost,
                        "durationMs": msg["duration_ms"],
                    }),
                );
                if let Some(rec) = self.snapshot(id) {
                    let mut d = Self::summary(&rec);
                    d["isError"] = json!(is_error);
                    d["costUsd"] = json!(rec.cost_usd);
                    d["usage"] = json!(rec.usage);
                    self.bus.emit(EVENT_TURN_END, d);

                    let (bus, sid, cwd) = (self.bus.clone(), rec.id.clone(), rec.cwd.clone());
                    std::thread::spawn(move || {
                        let count = crate::changes::changed_count(Path::new(&cwd));
                        bus.emit(
                            EVENT_CHANGES,
                            json!({ "sessionId": sid, "fileCount": count }),
                        );
                    });
                }
                if auto_close {
                    drop(lock(&live.stdin).take());
                }
            }
            MSG_RATE_LIMIT_EVENT => self.on_rate_limit(&msg),
            _ => self.push_event(
                id,
                live,
                json!({ "kind": SessionEventKind::Raw, "raw": msg }),
            ),
        }
    }

    /// Every workspace key with a live session; collects live ids first, then locks the index.
    /// This keeps `finish`'s index-then-live lock order.
    fn running_workspace_keys(&self) -> Vec<String> {
        let live_ids: Vec<String> = lock(&self.live).keys().cloned().collect();
        let mut keys: Vec<String> = {
            let idx = lock(&self.index);
            live_ids
                .into_iter()
                .filter_map(|id| idx.get(&id).map(workspace_key_of))
                .collect()
        };
        keys.extend(lock(&self.held_workspaces).keys().cloned());
        keys
    }

    /// True while a session runs in, or a writer holds, the workspace `workspace_key`.
    pub fn is_workspace_busy(&self, workspace_key: &str) -> bool {
        self.running_workspace_keys()
            .iter()
            .any(|key| key == workspace_key)
    }

    /// Reserves a workspace for a writer that is not a session.
    /// Returns false when a session runs there or another writer holds it.
    pub fn try_hold_workspace(&self, workspace_key: &str) -> bool {
        self.hold_workspace_as(workspace_key, None)
    }

    /// Like [`Self::try_hold_workspace`], recording the stage so a conflict can name it.
    pub fn try_hold_workspace_for_stage(&self, workspace_key: &str, stage: &str) -> bool {
        self.hold_workspace_as(workspace_key, Some(stage.to_string()))
    }

    fn hold_workspace_as(&self, workspace_key: &str, stage: Option<String>) -> bool {
        let _gate = lock(&self.admission);
        if self.is_workspace_busy(workspace_key) {
            return false;
        }
        lock(&self.held_workspaces).insert(workspace_key.to_string(), stage);
        true
    }

    /// Frees a workspace taken by [`Self::try_hold_workspace`] and replays a launch queued behind it.
    pub fn release_workspace(self: &Arc<Self>, workspace_key: &str) {
        lock(&self.held_workspaces).remove(workspace_key);
        self.dequeue_next(workspace_key, "");
    }

    /// The Claude transcript id to resume: a gateway session's, or an external one found on disk.
    fn resume_claude_id(
        &self,
        req: &StartRequest,
        project: &crate::projects::Project,
    ) -> anyhow::Result<Option<String>> {
        let external = req.external == Some(true);
        let Some(resume_from) = req.resume_from.as_deref() else {
            if external {
                bail!(MISSING_RESUME_ID_MESSAGE);
            }
            return Ok(None);
        };
        if external {
            let home = aidp_core::vault::home_dir().ok_or_else(|| {
                crate::session_error::SessionError::Config(NO_HOME_MESSAGE.to_string())
            })?;
            let found = crate::claude_sessions::find(&home, &project.path, resume_from)?;
            return Ok(Some(found.id));
        }
        self.snapshot(resume_from)
            .and_then(|record| record.provider_session_id.or(record.claude_session_id))
            .map(Some)
            .ok_or_else(|| anyhow!(crate::models::constants::session_resume::RESUME_MISSING))
    }

    /// Resolves an explicit workspace request; runs before the admission lock.
    fn resolve_requested_workspace(
        &self,
        requested: &WorkspaceRequest,
        project: &crate::projects::Project,
    ) -> anyhow::Result<ResolvedWorkspace> {
        let project_path = PathBuf::from(&project.path);
        let project_ref = ProjectRef {
            id: project.id.clone(),
            repo: aidp_core::vault::repo_segment(&project_path),
            path: project_path,
        };
        let project_paths: Vec<PathBuf> = self
            .projects
            .list()
            .iter()
            .map(|known| PathBuf::from(&known.path))
            .collect();
        let vault = self.configured_vault()?;
        let workspace = crate::workspace_resolver::resolve_workspace(
            self.runner.as_ref(),
            requested,
            &project_ref,
            &project_paths,
            vault.as_ref().map(|resolved| resolved.root.as_path()),
        )
        .map_err(|error| anyhow::Error::new(ApiRefusal::from(error)))?;
        Ok(ResolvedWorkspace {
            workspace,
            warnings: Vec::new(),
        })
    }

    /// Writer lanes and plan-lane keys of the live sessions; writers include held workspaces.
    fn running_lane_keys(&self) -> (Vec<WriterLane>, Vec<String>) {
        let live_ids: Vec<String> = lock(&self.live).keys().cloned().collect();
        let (mut writers, mut readers) = (Vec::new(), Vec::new());
        {
            let idx = lock(&self.index);
            let groups = lock(&self.session_writer_groups);
            for record in live_ids.iter().filter_map(|id| idx.get(id)) {
                let key = workspace_key_of(record);
                if record.permission_mode == PERMISSION_MODE_PLAN {
                    readers.push(key);
                } else {
                    writers.push((key, groups.get(&record.id).cloned()));
                }
            }
        }
        writers.extend(
            lock(&self.held_workspaces)
                .keys()
                .map(|key| (key.clone(), None)),
        );
        (writers, readers)
    }

    /// True when the session is no longer live and no other unarchived session shares its workspace.
    fn scratch_is_unused(&self, record: &SessionRecord) -> bool {
        if lock(&self.live).contains_key(&record.id) {
            return false;
        }
        let key = workspace_key_of(record);
        !lock(&self.index)
            .values()
            .any(|other| other.id != record.id && !other.archived && workspace_key_of(other) == key)
    }

    /// Removes the scratch worktree of an archived session, keeping unsaved work on a ref.
    fn release_scratch_workspace(&self, record: &SessionRecord) {
        let Some(workspace) = record
            .workspace
            .as_ref()
            .filter(|workspace| workspace.kind == WorkspaceKind::Scratch)
        else {
            return;
        };
        let Some(project) = self.projects.get(&record.project_id) else {
            return;
        };
        if let Err(error) = worktrees::release_scratch(
            self.runner.as_ref(),
            &thread::sleep,
            Path::new(&project.path),
            workspace,
        ) {
            warn!(session = %record.id, %error, "cannot release scratch workspace");
        }
    }

    /// The directory a native run of `req` works in, prepared like a session's
    /// (ticket worktree, requested workspace); `None` keeps the project checkout.
    pub fn native_workspace(&self, req: &StartRequest) -> anyhow::Result<Option<PathBuf>> {
        let Some(project) = self.projects.get(&req.project_id) else {
            return Ok(None);
        };
        let origin = req
            .origin
            .clone()
            .unwrap_or_else(|| Origin::new(OriginKind::Api.as_str()));
        let resolved = self.resolve_workspace(req, &origin, &project)?;
        let checkout = project_workspace(&project.path).path;
        Ok((resolved.workspace.path != checkout).then_some(resolved.workspace.path))
    }

    /// The workspace a launch runs in: the checkout, or a ticket worktree.
    ///
    /// Never called under the admission lock because it performs git I/O.
    fn resolve_workspace(
        &self,
        req: &StartRequest,
        origin: &Origin,
        project: &crate::projects::Project,
    ) -> anyhow::Result<ResolvedWorkspace> {
        if let Some(requested) = &req.workspace {
            return self.resolve_requested_workspace(requested, project);
        }
        if let Some(test) = flow_test_checkout(req, origin, project) {
            return Ok(ResolvedWorkspace {
                workspace: project_workspace(&test),
                warnings: Vec::new(),
            });
        }
        let checkout = project_workspace(&project.path);
        let unchanged = |warnings: Vec<String>| ResolvedWorkspace {
            workspace: checkout.clone(),
            warnings,
        };
        if origin.is(OriginKind::Ui) {
            return Ok(unchanged(Vec::new()));
        }
        let requested = req
            .worktree
            .as_deref()
            .map(str::trim)
            .filter(|key| !key.is_empty());
        let ci_feature = (origin.is(OriginKind::Ci))
            .then(|| req.inputs.as_ref()?.get(FEATURE_INPUT).cloned())
            .flatten();
        let Some(vault) = self.configured_vault()? else {
            let warnings = requested
                .map(|_| vec![WARNING_WORKTREE_NO_VAULT.to_string()])
                .unwrap_or_default();
            return Ok(unchanged(warnings));
        };
        let project_path = Path::new(&project.path);
        let repo = aidp_core::vault::repo_segment(project_path);
        if let Some(key) = requested {
            let copy = aidp_tickets::config::load(&vault.root)
                .dispatcher
                .worktree_copy;
            let base = aidp_tickets::store::TicketStore::open(&vault.root)
                .ok()
                .and_then(|store| worktrees::ticket_base(&store, key));
            let ensured = worktrees::ensure_based(
                self.runner.as_ref(),
                project_path,
                &vault.root,
                &repo,
                key,
                &copy,
                base.as_deref(),
            )
            .map_err(|error| match error {
                worktrees::EnsureError::BranchCheckedOutElsewhere { .. } => {
                    anyhow::Error::new(WorktreeRefused {
                        slug: error.slug(),
                        detail: error.to_string(),
                    })
                }
                other => anyhow!(other),
            })?;
            self.bus.emit(
                EVENT_WORKTREE_CREATED,
                json!({
                    FIELD_KEY: key,
                    FIELD_PROJECT: project.id,
                    FIELD_PATH: ensured.workspace.path,
                    FIELD_BRANCH: ensured.workspace.branch,
                    FIELD_REUSED: ensured.reused,
                }),
            );
            return Ok(ResolvedWorkspace {
                workspace: ensured.workspace,
                warnings: Vec::new(),
            });
        }
        let existing = ci_feature.and_then(|feature| {
            worktrees::existing_for_branch(
                self.runner.as_ref(),
                project_path,
                &vault.root,
                &repo,
                &worktrees::branch_for(&feature),
            )
        });
        Ok(match existing {
            Some(workspace) => ResolvedWorkspace {
                workspace,
                warnings: Vec::new(),
            },
            None => unchanged(Vec::new()),
        })
    }

    fn finish(self: &Arc<Self>, id: &str, live: &Live, code: Option<i32>) {
        let err = live.last_turn_error.load(Ordering::SeqCst);
        live.workflow_stop.store(true, Ordering::SeqCst);
        let rec = self.update(id, |r| {
            if r.status == SessionStatus::Running {
                r.status = if code == Some(0) && !err {
                    SessionStatus::Done
                } else {
                    SessionStatus::Failed
                };
            }
            r.exit_code = code;
            r.ended_at = Some(now_iso());
            r.pending_question = None;
            r.clone()
        });
        lock(&self.live).remove(id);
        lock(&self.session_writer_groups).remove(id);
        if let Some(rec) = rec {
            let mut d = Self::summary(&rec);
            d["exitCode"] = json!(code);
            d["costUsd"] = json!(rec.cost_usd);
            d["turns"] = json!(rec.turns);
            d["lastText"] = json!(rec.last_text);
            d["usage"] = json!(rec.usage);
            self.bus.emit(EVENT_SESSION_ENDED, d);
            self.run_end_hook(&rec);
            self.dequeue_next(&workspace_key_of(&rec), &rec.project_id);
        }
    }

    /// Takes and runs the hook registered for `rec`, outside every lock.
    fn run_end_hook(&self, rec: &SessionRecord) {
        let hook = lock(&self.end_hooks).remove(&rec.id);
        if let Some(hook) = hook {
            hook(rec);
        }
    }

    /// Runs `hook` once when session `id` ends; at once when it already ended.
    pub fn on_end(&self, id: &str, hook: SessionEndHook) {
        lock(&self.end_hooks).insert(id.to_string(), hook);
        if let Some(rec) = self.snapshot(id).filter(|rec| rec.ended_at.is_some()) {
            self.run_end_hook(&rec);
        }
    }

    /// Names what occupies `workspace_key`: a live session, else a held writer; `None` when free.
    pub fn workspace_holder(&self, workspace_key: &str) -> Option<String> {
        let live_ids: Vec<String> = lock(&self.live).keys().cloned().collect();
        let session = {
            let idx = lock(&self.index);
            live_ids.into_iter().find(|id| {
                idx.get(id)
                    .is_some_and(|rec| workspace_key_of(rec) == workspace_key)
            })
        };
        match session {
            Some(id) => Some(format!("{HOLDER_SESSION_PREFIX}{id}")),
            None => lock(&self.held_workspaces)
                .get(workspace_key)
                .map(|stage| match stage {
                    Some(name) => format!("{HOLDER_WORKFLOW_STAGE} {name}"),
                    None => HOLDER_WORKFLOW_STAGE.to_string(),
                }),
        }
    }

    /// Replay the next queued launch for `workspace_key` after its session ended.
    /// Entries without a workspace key wait on the project id; never panics.
    fn dequeue_next(self: &Arc<Self>, workspace_key: &str, project_id: &str) {
        if self.shutting_down.load(Ordering::SeqCst) {
            return;
        }
        let entry = {
            let gate = lock(&self.admission);
            let Some(candidate) = self.next_queued(workspace_key, project_id) else {
                return;
            };
            let (writer_keys, read_only_keys) = self.running_lane_keys();
            if crate::queue::admit_lane(
                workspace_key,
                queued_read_only(&candidate),
                queued_writer_group(&candidate).as_deref(),
                &writer_keys,
                &read_only_keys,
            ) == Admission::Queue
            {
                return;
            }
            let entry = self.queue.take_next(candidate.admission_key());
            drop(gate);
            entry
        };
        let Some(entry) = entry else {
            return;
        };
        let req = match serde_json::from_value::<StartRequest>(entry.request.clone()) {
            Ok(r) => r,
            Err(e) => {
                warn!("queued launch {} has an unparseable request: {e}", entry.id);
                self.bus.emit(
                    EVENT_DEQUEUED,
                    json!({
                        "queueId": entry.id,
                        "projectId": project_id,
                        "reason": DEQUEUE_FAILED,
                        "error": e.to_string(),
                    }),
                );
                return;
            }
        };
        match self.start(req) {
            Ok(Launch::Started(rec)) => {
                self.bus.emit(
                    EVENT_DEQUEUED,
                    json!({
                        "queueId": entry.id,
                        "projectId": project_id,
                        "reason": DEQUEUE_STARTED,
                        "sessionId": rec.id,
                    }),
                );
            }
            Ok(Launch::Queued(_)) => {
                warn!(
                    "queued launch {} for project {project_id} re-queued instead of starting",
                    entry.id
                );
                self.bus.emit(
                    EVENT_DEQUEUED,
                    json!({
                        "queueId": entry.id,
                        "projectId": project_id,
                        "reason": DEQUEUE_FAILED,
                        "error": REQUEUED_ERROR,
                    }),
                );
            }
            Err(e) => {
                error!(
                    "queued launch {} for project {project_id} failed to start: {e}",
                    entry.id
                );
                self.bus.emit(
                    EVENT_DEQUEUED,
                    json!({
                        "queueId": entry.id,
                        "projectId": project_id,
                        "reason": DEQUEUE_FAILED,
                        "error": e.to_string(),
                    }),
                );
            }
        }
    }

    /// The launch that [`Self::dequeue_next`] would take: one waiting on the workspace, else on the project.
    fn next_queued(&self, workspace_key: &str, project_id: &str) -> Option<QueuedLaunch> {
        let waiting = self.queue.list(None);
        [workspace_key, project_id].into_iter().find_map(|key| {
            waiting
                .iter()
                .find(|entry| entry.admission_key() == key)
                .cloned()
        })
    }

    pub fn queue_list(&self, project_id: Option<&str>) -> Vec<Value> {
        self.queue
            .list(project_id)
            .iter()
            .map(QueuedLaunch::public)
            .collect()
    }

    pub fn queue_depth(&self) -> usize {
        self.queue.depth()
    }

    pub fn queue_promote(&self, id: &str) -> Option<Value> {
        self.queue.promote(id).map(|q| q.public())
    }

    pub fn queue_cancel(&self, id: &str) -> Option<Value> {
        self.queue.cancel(id).map(|q| q.public())
    }

    #[tracing::instrument(
        skip_all,
        fields(run_id = tracing::field::Empty, ticket_key = tracing::field::Empty, session_id = tracing::field::Empty)
    )]
    pub fn start(self: &Arc<Self>, mut req: StartRequest) -> anyhow::Result<Launch> {
        crate::claude_provider::validate_policy_spec(&AgentSpec {
            mcp_servers: req.mcp_servers.clone(),
            denied_shell_commands: req.denied_shell_commands.clone(),
            denied_mcp_tools: req.denied_mcp_tools.clone(),
            ..AgentSpec::default()
        })?;
        if req.external != Some(true) {
            if let Some(parent) = req.resume_from.as_deref().and_then(|id| self.snapshot(id)) {
                let parent_provider = parent.provider.as_deref().unwrap_or(CLAUDE_PROVIDER);
                if req
                    .provider
                    .as_deref()
                    .is_some_and(|provider| provider != parent_provider)
                {
                    bail!(crate::models::constants::session_resume::RESUME_PROVIDER_MISMATCH);
                }
                req.provider = Some(parent_provider.to_string());
                if req.mcp_config_id.is_none() {
                    req.mcp_config_id = parent.mcp_config_id;
                }
                req.headless |= parent.headless;
            }
        }
        let mcp_configuration = self
            .mcp_configurations
            .resolve(req.mcp_config_id.as_deref())?;
        let project = self.projects.get(&req.project_id).ok_or_else(|| {
            crate::session_error::SessionError::NotFound("project not found".to_string())
        })?;
        let engine = req
            .engine
            .clone()
            .filter(|e| !e.is_empty())
            .unwrap_or_else(|| {
                if req.template_id.is_some() {
                    ENGINE_WORKFLOW.to_string()
                } else {
                    ENGINE_PROMPT.to_string()
                }
            });
        if engine != ENGINE_PROMPT
            && req
                .template_id
                .as_deref()
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .is_none()
        {
            bail!("templateId is required");
        }
        if engine == ENGINE_PROMPT
            && req
                .prompt
                .as_deref()
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .is_none()
        {
            bail!("prompt is required");
        }
        if let Some(m) = req.permission_mode.as_deref().filter(|m| !m.is_empty()) {
            validate_permission_mode(m)?;
        }
        if let Some(e) = req.effort.as_deref().filter(|e| !e.is_empty()) {
            validate_effort(e)?;
        }
        if let Some(m) = req.model.as_deref().filter(|m| !m.is_empty()) {
            validate_model(m)?;
        }
        let resume_claude_id = self.resume_claude_id(&req, &project)?;
        if let Some(id) = resume_claude_id.as_deref() {
            validate_claude_session_id(id)?;
        }

        let id = new_id();
        let span = tracing::Span::current();
        span.record(FIELD_SESSION_ID, id.as_str());
        if let Some(ticket_key) = req.origin.as_ref().and_then(|o| o.r#ref.as_deref()) {
            span.record(FIELD_TICKET_KEY, ticket_key);
        }

        let mut chain_depth: Option<u32> = None;
        let mut chain_block = String::new();
        let mut resolved_max_budget_usd = req.max_budget_usd;
        if let Some(pid) = &req.context_from {
            if let Some(parent) = self.snapshot(pid) {
                let mut ancestry: Vec<String> = Vec::new();
                let mut cursor = Some(parent.clone());
                let mut steps = 0u32;
                while let Some(cur) = cursor {
                    ancestry.push(cur.id.clone());
                    steps += 1;
                    if steps > max_chain_depth() + 2 {
                        break;
                    }
                    cursor = match &cur.context_from {
                        Some(next_id) => self.snapshot(next_id),
                        None => None,
                    };
                }

                match decide_chain(&id, &ancestry, parent.chain_depth.unwrap_or(0)) {
                    ChainDecision::Cycle(cycle_id) => {
                        bail!("cannot chain: cycle through session {cycle_id}");
                    }
                    ChainDecision::TooDeep { depth, max } => {
                        bail!("cannot chain: depth {depth} exceeds the maximum {max}");
                    }
                    ChainDecision::Admit { depth } => {
                        chain_depth = Some(depth);
                        resolved_max_budget_usd =
                            inherit_budget(parent.max_budget_usd, req.max_budget_usd);
                        if let Some(run_ref) =
                            crate::runs::resolve_run(&parent.cwd, parent.workflow_run_id.as_deref())
                        {
                            if !crate::runs::run_is_gate_blocked(&run_ref.run_dir) {
                                if let Some(artefacts) =
                                    crate::runs::read_chain_artefacts(&run_ref.run_dir)
                                {
                                    chain_block = crate::runs::build_context_block(&artefacts);
                                }
                            }
                        }
                    }
                }
            }
        }

        let generation = self
            .runtime_environment()
            .get(SKILL_ROOT_ENV)
            .map(|root| crate::paths::configured_skill_generation(Path::new(root)))
            .unwrap_or_else(skill_generation);
        let prompt = build_prompt(&req, &engine, &generation);
        let prompt = if chain_block.is_empty() {
            prompt
        } else {
            format!("{prompt}\n\n{chain_block}")
        };
        let mode = launch_permission_mode(req.permission_mode.as_deref(), req.headless);
        let origin = req
            .origin
            .clone()
            .unwrap_or_else(|| Origin::new(OriginKind::Api.as_str()));
        let auto_close = req.auto_close.unwrap_or(!origin.is(OriginKind::Ui));

        let (provider, launch_model) = self.resolve_launch_provider(
            req.model.as_deref().filter(|m| !m.is_empty()),
            req.provider.as_deref().filter(|p| !p.is_empty()),
        )?;
        let launch_tail = claude_launch_tail(
            &mode,
            launch_model.as_deref(),
            req.effort.as_deref(),
            resolved_max_budget_usd,
            resume_claude_id.as_deref(),
        );
        let profile = SessionProfile::from_env(&req.env);
        let launch_tail = profile.apply_to_launch_tail_for_push_branch(
            launch_tail,
            req.env.get(ALLOWED_PUSH_BRANCH_ENV).map(String::as_str),
        )?;
        let launch_tail = with_allowed_tools(launch_tail, &req.allowed_tools);
        let launch_tail = with_add_dirs(launch_tail, &req.add_dirs);

        let resolved = self.resolve_workspace(&req, &origin, &project)?;
        let workspace = resolved.workspace;
        let workspace_key = workspace.key();
        let start_sha = crate::changes::head(&workspace.path);
        let gates: Vec<&str> = req
            .auto_approve_gates
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(String::as_str)
            .filter(|g| !g.trim().is_empty())
            .collect();
        let mut extra_env: Vec<(String, String)> = Vec::new();
        if !gates.is_empty() {
            extra_env.push((AUTO_APPROVE_GATES_ENV.to_string(), gates.join(",")));
        }
        let runtime_environment = self.runtime_environment();
        let configured_binary = runtime_environment.get(crate::aidp_bin::AIDP_BIN_ENV);
        if let Some(binary) = configured_binary {
            extra_env.extend(crate::aidp_bin::resolve_child_env(
                Some(binary.into()),
                std::env::current_exe(),
                runtime_environment
                    .get(crate::aidp_bin::PATH_ENV)
                    .map(Into::into),
            )?);
        } else {
            extra_env.extend(crate::aidp_bin::child_env_from_process()?);
        }
        extra_env.extend(req.env.iter().map(|(k, v)| (k.clone(), v.clone())));
        if let Some(environment) = lock(&self.runtime_environment).as_ref() {
            extra_env.extend(
                environment
                    .iter()
                    .map(|(key, value)| (key.clone(), value.clone())),
            );
        }
        if req.headless {
            use crate::models::constants::launch_policy::{
                HEADLESS_SESSION_ENV, HEADLESS_SESSION_VALUE,
            };
            extra_env.push((
                HEADLESS_SESSION_ENV.to_string(),
                HEADLESS_SESSION_VALUE.to_string(),
            ));
        }

        let one_session_per_project_gate = lock(&self.admission);
        if self.shutting_down.load(Ordering::SeqCst) {
            drop(one_session_per_project_gate);
            return Err(crate::session_error::SessionError::ShuttingDown.into());
        }
        let read_only = mode == PERMISSION_MODE_PLAN;
        let (writer_keys, read_only_keys) = self.running_lane_keys();
        let writer_group = req.shared_writer_group.as_deref();
        if crate::queue::admit_lane(
            &workspace_key,
            read_only,
            writer_group,
            &writer_keys,
            &read_only_keys,
        ) == Admission::Queue
        {
            drop(one_session_per_project_gate);
            let entry = QueuedLaunch {
                id: new_id(),
                project_id: project.id.clone(),
                project_name: project.name.clone(),
                template_id: req.template_id.clone(),
                target: req.target.clone(),
                engine: engine.clone(),
                priority: req.priority.unwrap_or(0),
                status: SessionStatus::Queued.as_str().to_string(),
                queued_at: now_iso(),
                origin: origin.clone(),
                workspace_key: Some(workspace_key),
                reason: Some(crate::queue::queue_reason(read_only)),
                request: serde_json::to_value(&req)?,
            };
            return Ok(Launch::Queued(Box::new(self.queue.enqueue(entry))));
        }

        let agent_spec = AgentSpec {
            mcp_configuration,
            work_dir: workspace.path.clone(),
            prompt: prompt.clone(),
            model: launch_model,
            effort: req.effort.clone(),
            env: extra_env.iter().cloned().collect(),
            extra_args: launch_tail,
            session_id: resume_claude_id,
            strict_mcp_config: req.strict_mcp_config,
            mcp_servers: req.mcp_servers.clone(),
            denied_shell_commands: req.denied_shell_commands.clone(),
            denied_mcp_tools: req.denied_mcp_tools.clone(),
            permission_policy: permission_policy(&mode, req.headless),
            ..AgentSpec::default()
        };
        let (handle, stdin) = self.launch_agent(&provider, agent_spec)?;

        let rec = SessionRecord {
            mcp_config_id: req.mcp_config_id.clone(),
            id: id.clone(),
            provider: Some(provider.id().to_string()),
            provider_session_id: None,
            project_id: project.id.clone(),
            project_name: project.name.clone(),
            cwd: workspace.path.to_string_lossy().to_string(),
            engine: engine.clone(),
            template_id: req.template_id.clone(),
            target: req.target.clone(),
            inputs: req.inputs.clone(),
            prompt: prompt.clone(),
            permission_mode: mode,
            effective_permission_mode: None,
            model: req.model.clone(),
            effort: req.effort.clone(),
            max_budget_usd: resolved_max_budget_usd,
            status: SessionStatus::Running,
            started_at: now_iso(),
            ended_at: None,
            exit_code: None,
            claude_session_id: None,
            resumed_from: req.resume_from.clone(),
            cost_usd: None,
            turns: 0,
            last_text: None,
            pending_question: None,
            origin,
            automation_id: req.automation_id.clone(),
            automation_run_id: req.automation_run_id.clone(),
            workflow_run_id: None,
            event_count: 0,
            auto_close,
            usage: None,
            title: resolve_title(&req),
            pinned: false,
            archived: false,
            unread: false,
            start_sha,
            context_from: req.context_from.clone(),
            chain_depth,
            workspace: Some(workspace),
            warnings: resolved.warnings,
            headless: req.headless,
        };
        let live = Arc::new(Live {
            stdin: Mutex::new(None),
            handle: Mutex::new(Some(handle)),
            events: Mutex::new(Vec::new()),
            last_turn_error: AtomicBool::new(false),
            workflow_tool_uses: Mutex::new(HashSet::new()),
            workflow_runs: Mutex::new(HashSet::new()),
            workflow_stop: Arc::new(AtomicBool::new(false)),
            permission_requests: Mutex::new(std::collections::VecDeque::new()),
        });
        if let Some(group) = req
            .shared_writer_group
            .as_ref()
            .filter(|name| !name.is_empty())
        {
            lock(&self.session_writer_groups).insert(id.clone(), group.clone());
        }
        lock(&self.index).insert(id.clone(), rec.clone());
        lock(&self.live).insert(id.clone(), live.clone());
        drop(one_session_per_project_gate);

        self.save(&rec);
        self.bus.emit(EVENT_STARTED, Self::summary(&rec));

        {
            let (mgr, live2, id2) = (self.clone(), live.clone(), id.clone());
            thread::spawn(move || {
                let code = mgr.pump_agent_events(&id2, &live2);
                mgr.finish(&id2, &live2, code);
            });
        }
        if let Some(stdin) = stdin {
            self.send_initial_prompt(&id, &live, stdin, &prompt);
        }
        Ok(Launch::Started(Box::new(rec)))
    }

    /// Write the first prompt to the child's stdin, then hand stdin to `live`.
    /// Runs after the reader threads start so a child writing first cannot block.
    fn send_initial_prompt(
        &self,
        id: &str,
        live: &Live,
        mut stdin: Box<dyn Write + Send>,
        prompt: &str,
    ) {
        let first = user_message_line(prompt);
        if let Err(e) = stdin
            .write_all(first.as_bytes())
            .and_then(|_| stdin.flush())
        {
            self.push_event(id, live, json!({ "kind": SessionEventKind::Error, "message": format!("Failed to send initial prompt: {e}") }));
        }
        *lock(&live.stdin) = Some(stdin);
    }

    pub fn get(&self, id: &str) -> Option<SessionRecord> {
        self.snapshot(id)
    }

    /// Shallow update of `title`, `pinned`, `archived`, `unread`; unknown keys are ignored.
    /// A null or empty `title` clears it; `None` means unknown id.
    pub fn patch(&self, id: &str, body: &Value) -> Option<SessionRecord> {
        let was_archived = self.snapshot(id).map(|record| record.archived);
        let rec = self.update(id, |r| {
            if let Some(v) = body.get("title") {
                r.title = v
                    .as_str()
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty());
            }
            if let Some(b) = body.get("pinned").and_then(Value::as_bool) {
                r.pinned = b;
            }
            if let Some(b) = body.get("archived").and_then(Value::as_bool) {
                r.archived = b;
            }
            if let Some(b) = body.get("unread").and_then(Value::as_bool) {
                r.unread = b;
            }
            r.clone()
        })?;
        if was_archived == Some(false) && rec.archived && self.scratch_is_unused(&rec) {
            self.release_scratch_workspace(&rec);
        }
        self.bus.emit(EVENT_UPDATED, Self::summary(&rec));
        Some(rec)
    }

    pub fn list(
        &self,
        status: Option<SessionStatus>,
        project_id: Option<&str>,
        limit: usize,
    ) -> Vec<SessionRecord> {
        self.search(status, project_id, None, limit)
    }

    /// Same filters as `list`, plus a free-text `q` over title/template/target/projectName.
    pub fn search(
        &self,
        status: Option<SessionStatus>,
        project_id: Option<&str>,
        q: Option<&str>,
        limit: usize,
    ) -> Vec<SessionRecord> {
        let mut all: Vec<SessionRecord> = lock(&self.index)
            .values()
            .filter(|s| status.map(|st| s.status == st).unwrap_or(true))
            .filter(|s| project_id.map(|p| s.project_id == p).unwrap_or(true))
            .filter(|s| q.map(|n| matches_q(s, n)).unwrap_or(true))
            .cloned()
            .collect();
        all.sort_by(|a, b| b.started_at.cmp(&a.started_at));
        all.truncate(if limit == 0 { 200 } else { limit });
        all
    }

    pub fn running_count(&self) -> usize {
        lock(&self.live).len()
    }

    pub fn is_running(&self, id: &str) -> bool {
        lock(&self.live).contains_key(id)
    }

    /// All events so far — live buffer for a running session, JSONL otherwise.
    pub fn events(&self, id: &str) -> Vec<Value> {
        if let Some(l) = lock(&self.live).get(id).cloned() {
            return lock(&l.events).clone();
        }
        if lock(&self.index).contains_key(id) {
            read_jsonl(&self.events_file(id))
        } else {
            Vec::new()
        }
    }

    /// Markdown transcript for `GET /api/sessions/:id/export.md`. `None` for
    /// an unknown id; the caller turns that into a 404.
    pub fn export_markdown(&self, id: &str) -> Option<String> {
        let rec = self.get(id)?;
        let events = self.events(id);
        Some(render_markdown(&rec, &events))
    }

    fn live_of(&self, id: &str) -> anyhow::Result<Arc<Live>> {
        lock(&self.live)
            .get(id)
            .cloned()
            .ok_or_else(|| anyhow!("session not running"))
    }

    fn write_line(&self, live: &Live, v: Value) -> anyhow::Result<()> {
        let mut g = lock(&live.stdin);
        let s = g.as_mut().ok_or_else(|| anyhow!("session input closed"))?;
        s.write_all((v.to_string() + "\n").as_bytes())?;
        s.flush()?;
        Ok(())
    }

    /// `{ text }` for a free message, `{ answers: { "<question>": { answer, note? } } }` for a pending gate.
    pub fn reply(&self, id: &str, body: &Value) -> anyhow::Result<()> {
        let live = self.live_of(id)?;
        let rec = self
            .snapshot(id)
            .ok_or_else(|| anyhow!("session not found"))?;
        if let Some(expected) = body.get(crate::models::constants::question_reply::TOOL_USE_ID) {
            let pending_id = rec
                .pending_question
                .as_ref()
                .map(|question| question.tool_use_id.as_str());
            if expected.as_str().is_none() || expected.as_str() != pending_id {
                return Err(AgentError::Config(
                    crate::models::constants::question_reply::STALE_QUESTION.to_string(),
                )
                .into());
            }
        }
        if let Some(request_id) = rec
            .pending_question
            .as_ref()
            .and_then(|question| question.permission_request_id.as_deref())
        {
            use crate::models::constants::claude_permissions;
            let answer = body
                .get("answers")
                .and_then(|answers| answers.get(claude_permissions::QUESTION))
                .and_then(|answer| answer.get("answer"))
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    AgentError::Config(claude_permissions::INVALID_ANSWER.to_string())
                })?;
            let allow = match answer {
                claude_permissions::ALLOW => true,
                claude_permissions::DENY => false,
                _ => {
                    return Err(
                        AgentError::Config(claude_permissions::INVALID_ANSWER.to_string()).into(),
                    )
                }
            };
            let handle = lock(&live.handle);
            handle
                .as_ref()
                .ok_or_else(|| anyhow!(claude_permissions::INVALID_REQUEST))?
                .reply_permission(request_id, allow)?;
            {
                let mut requests = lock(&live.permission_requests);
                requests.pop_front();
                self.update(id, |record| record.pending_question = None);
            }
            self.push_event(id, &live, json!({ "kind": SessionEventKind::UserReply, "toolUseId": request_id, "answers": body["answers"] }));
            if let Some(record) = self.snapshot(id) {
                self.bus.emit(EVENT_ANSWERED, Self::summary(&record));
            }
            self.show_hosted_permission(id, &live);
            return Ok(());
        }
        let attachments = parse_attachments(body)?;
        let image_blocks = crate::attachments::validate_and_build(&attachments)
            .map_err(|error| anyhow::Error::new(ApiRefusal::from(error)))?;
        if let (Some(pq), Some(answers)) = (
            &rec.pending_question,
            body.get("answers").filter(|a| a.is_object()),
        ) {
            let empty = Vec::new();
            let lines: Vec<String> = pq
                .questions
                .as_array()
                .unwrap_or(&empty)
                .iter()
                .enumerate()
                .map(|(i, q)| {
                    let qt = q["question"].as_str().unwrap_or("");
                    match answers.get(qt).or_else(|| answers.get(i.to_string())) {
                        Some(a) => format!(
                            "- {qt}\n  → {}{}",
                            a["answer"].as_str().unwrap_or(""),
                            a["note"]
                                .as_str()
                                .map(|n| format!("\n  note: {n}"))
                                .unwrap_or_default()
                        ),
                        None => format!("- {qt}\n  (no answer)"),
                    }
                })
                .collect();
            self.write_line(
                &live,
                json!({
                    "type": StreamMessageType::User,
                    "message": { "role": MessageRole::User, "content": [{
                        "type": BLOCK_TOOL_RESULT,
                        "tool_use_id": pq.tool_use_id,
                        "content": format!("User answers:\n{}", lines.join("\n")),
                    }] }
                }),
            )?;
            self.push_event(
                id,
                &live,
                json!({ "kind": SessionEventKind::UserReply, "toolUseId": pq.tool_use_id, "answers": answers }),
            );
            self.update(id, |r| r.pending_question = None);
            if let Some(rec) = self.snapshot(id) {
                self.bus.emit(EVENT_ANSWERED, Self::summary(&rec));
            }
            self.show_hosted_permission(id, &live);
            return Ok(());
        }
        let text = body["text"].as_str().filter(|t| !t.trim().is_empty());
        if text.is_some() || !image_blocks.is_empty() {
            let message = UserStreamMessage {
                message_type: StreamMessageType::User,
                message: UserMessageBody {
                    role: MessageRole::User,
                    content: reply_content(text, &image_blocks),
                },
            };
            self.write_line(&live, serde_json::to_value(message)?)?;
            let event = UserMessageEvent {
                kind: SessionEventKind::UserMessage,
                text: text.unwrap_or_default().to_string(),
                attachments: image_blocks.len(),
            };
            self.push_event(id, &live, serde_json::to_value(event)?);
            return Ok(());
        }
        bail!("text or answers required")
    }

    /// Interrupt the current turn (stream-json control request), keeping the session alive.
    pub fn interrupt(&self, id: &str) -> anyhow::Result<()> {
        let live = self.live_of(id)?;
        self.write_line(
            &live,
            json!({ "type": CONTROL_REQUEST, "request_id": new_id(), "request": { "subtype": INTERRUPT_SUBTYPE } }),
        )?;
        self.push_event(
            id,
            &live,
            json!({ "kind": SessionEventKind::Log, "text": INTERRUPTED_LOG, "stream": SYSTEM_STREAM }),
        );
        Ok(())
    }

    pub fn cancel(&self, id: &str) -> anyhow::Result<()> {
        let live = self.live_of(id)?;
        drop(lock(&live.stdin).take());
        Self::cancel_agent(&live)?;
        self.update(id, |r| r.status = SessionStatus::Cancelled);
        Ok(())
    }

    pub fn delete(&self, id: &str) -> bool {
        if self.is_running(id) {
            let _ = self.cancel(id);
        }
        if lock(&self.index).remove(id).is_none() {
            return false;
        }
        for f in [self.meta_file(id), self.events_file(id)] {
            let _ = std::fs::remove_file(f);
        }
        true
    }

    /// Drop every `orphaned` session and return the purged ids.
    /// Never call `self.delete()` here: it locks the index and would deadlock.
    pub fn purge_orphaned(&self) -> Vec<String> {
        let mut ids: Vec<String> = {
            let mut idx = lock(&self.index);
            let ids: Vec<String> = idx
                .values()
                .filter(|r| r.status == SessionStatus::Orphaned)
                .map(|r| r.id.clone())
                .collect();
            for id in &ids {
                idx.remove(id);
            }
            ids
        };
        ids.sort();
        if ids.is_empty() {
            return ids;
        }
        for id in &ids {
            for f in [self.meta_file(id), self.events_file(id)] {
                let _ = std::fs::remove_file(f);
            }
        }
        self.bus.emit(
            EVENT_PURGED,
            json!({ "count": ids.len(), "sessionIds": ids }),
        );
        ids
    }

    /// Mark the manager as shutting down and kill every live child.
    /// Holds the `admission` gate so a just-admitted session is fully registered first.
    pub fn shutdown(&self) {
        if let Err(error) = self.try_shutdown() {
            warn!(%error, "session manager shutdown could not confirm every agent stopped");
        }
    }

    /// Confirms agent cancellation and exit, retaining failed live handles for a retry.
    pub fn try_shutdown(&self) -> anyhow::Result<()> {
        let gate = lock(&self.admission);
        self.shutting_down.store(true, Ordering::SeqCst);
        let ids: Vec<String> = lock(&self.live).keys().cloned().collect();
        drop(gate);
        let mut lives = Vec::new();
        for id in ids {
            if let Ok(live) = self.live_of(&id) {
                lives.push((id, live));
            }
        }
        let failures = Mutex::new(Vec::new());
        thread::scope(|scope| {
            for (id, live) in &lives {
                let failures = &failures;
                scope.spawn(move || match Self::cancel_agent(live) {
                    Ok(()) => {
                        self.update(id, |record| {
                            record.status = SessionStatus::Orphaned;
                            record.ended_at = Some(now_iso());
                        });
                        lock(&live.handle).take();
                    }
                    Err(error) => {
                        lock(failures).push(format!("{id}: {error}"));
                    }
                });
            }
        });
        let failures = lock(&failures);
        anyhow::ensure!(failures.is_empty(), "{}", failures.join("; "));
        Ok(())
    }
}

#[cfg(test)]
#[path = "../tests/unit/sessions_tests.rs"]
mod sessions_tests;

#[cfg(test)]
#[path = "../tests/unit/hosted_permissions_tests.rs"]
mod hosted_permissions_tests;

#[cfg(test)]
#[path = "../tests/unit/worktree_admission_tests.rs"]
mod worktree_admission_tests;

#[cfg(test)]
#[path = "../tests/unit/session_fake_provider_tests.rs"]
mod session_fake_provider_tests;

#[cfg(test)]
#[path = "../tests/unit/session_shutdown_tests.rs"]
mod session_shutdown_tests;
