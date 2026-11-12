//! Claude Code provider: command-line construction and stream-json events over a [`ProcessRunner`].

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread;
use std::time::Duration;

use aidp_agents::{
    AgentControl, AgentError, AgentEvent, AgentHandle, AgentProvider, AgentSpec, Capabilities,
    ExitReason, PermissionPolicy, TokenUsage,
};
use aidp_core::process_runner::{ProcessRunner, ProcessSpec, SpawnedChild};
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::models::claude_control::ClaudeControl;
use crate::models::enums::{
    ContentBlockType, ControlSubtype, MessageRole, PermissionBehavior, StreamMessageType,
};
use crate::models::exit_signal::ExitSignal;
pub use crate::models::{ClaudeCodeProvider, ClaudeLaunch, StdinGuard};
use crate::models::{
    ControlResponseBody, ControlResponseLine, PermissionDecision, ReplyContent, UserMessageBody,
    UserStreamMessage,
};
use crate::sync::lock;

/// Provider id of the Claude Code CLI.
pub const CLAUDE_PROVIDER_ID: &str = "claude";
pub(crate) const CLAUDE_PROGRAM: &str = "claude";
pub(crate) const BASE_CLAUDE_ARGS: &[&str] = &[
    "-p",
    "--output-format",
    "stream-json",
    "--input-format",
    "stream-json",
    "--verbose",
    "--permission-mode",
];
const MODEL_FLAG: &str = "--model";
const EFFORT_FLAG: &str = "--effort";
const MAX_BUDGET_FLAG: &str = "--max-budget-usd";
const RESUME_FLAG: &str = "--resume";
const PERSONA_FLAG: &str = "--append-system-prompt";
const NO_STDOUT_MESSAGE: &str = "no stdout";
const INPUT_CLOSED_MESSAGE: &str = "session input closed";
const CHILD_EXIT_POLL_INTERVAL: Duration = Duration::from_millis(100);
const DENY_MESSAGE: &str = "denied by the gateway";
const FIELD_TYPE: &str = "type";
const FIELD_MESSAGE: &str = "message";
const FIELD_CONTENT: &str = "content";
const FIELD_USAGE: &str = "usage";
const FIELD_REQUEST: &str = "request";
const FIELD_REQUEST_ID: &str = "request_id";
const FIELD_SUBTYPE: &str = "subtype";
const FIELD_TOOL_NAME: &str = "tool_name";
const FIELD_INPUT: &str = "input";
const FIELD_ID: &str = "id";
const FIELD_NAME: &str = "name";
const FIELD_TEXT: &str = "text";
const FIELD_TOOL_USE_ID: &str = "tool_use_id";
const FIELD_IS_ERROR: &str = "is_error";
const USAGE_INPUT: &str = "input_tokens";
const USAGE_OUTPUT: &str = "output_tokens";
const USAGE_CACHE_CREATION: &str = "cache_creation_input_tokens";
const USAGE_CACHE_READ: &str = "cache_read_input_tokens";

pub(crate) type ChildCell = Arc<Mutex<Option<Box<dyn SpawnedChild>>>>;
pub(crate) type StdinCell = Arc<Mutex<Option<Box<dyn Write + Send>>>>;
pub(crate) type PermissionCell = Arc<Mutex<BTreeMap<String, Value>>>;

pub(crate) fn validate_policy_spec(spec: &AgentSpec) -> Result<(), AgentError> {
    use crate::models::constants::{flow_execution, launch_policy};
    if let Some(configuration) = &spec.mcp_configuration {
        configuration.validate()?;
    }
    if spec
        .mcp_servers
        .as_ref()
        .is_some_and(|servers| !servers.is_empty())
    {
        return Err(AgentError::Config(
            flow_execution::UNSUPPORTED_MCP_DEFINITIONS.to_string(),
        ));
    }
    let safe = |value: &str| {
        !value.trim().is_empty()
            && value.chars().all(|character| {
                character.is_ascii_alphanumeric()
                    || matches!(character, ' ' | '-' | '_' | '.' | '/')
            })
    };
    if spec
        .denied_shell_commands
        .iter()
        .any(|command| !safe(command))
        || spec.denied_mcp_tools.iter().any(|(server, tools)| {
            // `*` alone denies every tool of the server.
            !safe(server) || tools.iter().any(|tool| tool != "*" && !safe(tool))
        })
    {
        return Err(AgentError::Config(
            launch_policy::INVALID_DENY_RULE.to_string(),
        ));
    }
    Ok(())
}

/// The persona of a session, followed by the denial guidance when the session is headless.
fn headless_persona(persona: Option<&str>, headless: bool) -> Option<String> {
    if !headless {
        return persona.map(str::to_string);
    }
    let guidance = crate::models::constants::launch_policy::HEADLESS_DENIAL_GUIDANCE;
    Some(match persona {
        Some(persona) => format!(
            "{persona}

{guidance}"
        ),
        None => guidance.to_string(),
    })
}

fn restricted_claude_args(spec: &AgentSpec) -> Result<Vec<String>, AgentError> {
    validate_policy_spec(spec)?;
    let headless = spec.permission_policy == PermissionPolicy::Headless;
    let persona = headless_persona(spec.persona.as_deref(), headless);
    let mut args = claude_args(&spec.extra_args, persona.as_deref());
    if spec.strict_mcp_config == Some(true)
        || spec.mcp_servers.is_some()
        || spec.mcp_configuration.is_some()
    {
        args.push(crate::models::constants::launch_policy::STRICT_MCP_FLAG.to_string());
    }
    let headless_denied: &[&str] = if headless {
        crate::models::constants::launch_policy::HEADLESS_DENIED_COMMANDS
    } else {
        &[]
    };
    let mut deny_rules = Vec::new();
    for command in spec
        .denied_shell_commands
        .iter()
        .map(String::as_str)
        .chain(headless_denied.iter().copied())
    {
        deny_rules.extend([
            format!("Bash({command}:*)"),
            format!("Bash(rtk {command}:*)"),
        ]);
    }
    for (server, tools) in &spec.denied_mcp_tools {
        deny_rules.extend(tools.iter().map(|tool| format!("mcp__{server}__{tool}")));
    }
    if !deny_rules.is_empty() {
        let flag = crate::models::constants::session_profile::DISALLOWED_TOOLS_FLAG;
        if let Some(position) = args.iter().position(|argument| argument == flag) {
            let end = args[position + 1..]
                .iter()
                .position(|argument| argument.starts_with("--"))
                .map_or(args.len(), |offset| position + 1 + offset);
            args.splice(end..end, deny_rules);
        } else {
            args.push(flag.to_string());
            args.extend(deny_rules);
        }
    }
    Ok(args)
}

/// The restricted argv of `spec`, relaying permission prompts over stdio only when its policy asks.
fn claude_session_args(spec: &AgentSpec) -> Result<Vec<String>, AgentError> {
    let mut args = restricted_claude_args(spec)?;
    if matches!(
        spec.permission_policy,
        PermissionPolicy::Default | PermissionPolicy::Stdio
    ) {
        args.extend([
            crate::models::constants::claude_permissions::PROMPT_TOOL_FLAG.to_string(),
            crate::models::constants::claude_permissions::STDIO.to_string(),
        ]);
    }
    Ok(args)
}

/// The argv tail after [`BASE_CLAUDE_ARGS`]: permission mode, then model, effort, budget, resume.
pub(crate) fn claude_launch_tail(
    permission_mode: &str,
    model: Option<&str>,
    effort: Option<&str>,
    max_budget_usd: Option<f64>,
    resume_session_id: Option<&str>,
) -> Vec<String> {
    let mut tail = vec![permission_mode.to_string()];
    if let Some(model) = model.filter(|model| !model.is_empty()) {
        tail.extend([MODEL_FLAG.to_string(), model.to_string()]);
    }
    if let Some(effort) = effort.filter(|effort| !effort.is_empty()) {
        tail.extend([EFFORT_FLAG.to_string(), effort.to_string()]);
    }
    if let Some(budget) = max_budget_usd.filter(|budget| *budget > 0.0) {
        tail.extend([MAX_BUDGET_FLAG.to_string(), budget.to_string()]);
    }
    if let Some(resume) = resume_session_id {
        tail.extend([RESUME_FLAG.to_string(), resume.to_string()]);
    }
    tail
}

/// The full `claude` argv: base flags, the caller's tail, then the persona when one is set.
pub(crate) fn claude_args(tail: &[String], persona: Option<&str>) -> Vec<String> {
    let mut args: Vec<String> = BASE_CLAUDE_ARGS.iter().map(|arg| arg.to_string()).collect();
    args.extend(tail.iter().cloned());
    if let Some(persona) = persona {
        args.extend([PERSONA_FLAG.to_string(), persona.to_string()]);
    }
    args
}

/// The process description handed to the runner: piped stdio, workspace cwd, extra environment.
pub(crate) fn claude_process_spec(
    args: Vec<String>,
    cwd: &Path,
    env: &BTreeMap<String, String>,
) -> ProcessSpec {
    let mut spec = ProcessSpec::new(CLAUDE_PROGRAM).args(args);
    spec.cwd = Some(cwd.to_path_buf());
    spec.piped_stdin = true;
    spec.piped_stdout = true;
    spec.piped_stderr = true;
    for (key, value) in env {
        spec.envs.insert(key.clone(), value.clone());
    }
    spec
}

/// One stream-json line carrying `prompt` as a user message.
pub(crate) fn user_message_line(prompt: &str) -> String {
    let line = UserStreamMessage {
        message_type: StreamMessageType::User,
        message: UserMessageBody {
            role: MessageRole::User,
            content: ReplyContent::Text(prompt.to_string()),
        },
    };
    serde_json::to_string(&line).unwrap_or_default()
        + "
"
}

/// Wait for the child to exit, holding its mutex only for each poll so a cancel can still kill it.
pub(crate) fn wait_for_child(child: &Mutex<Option<Box<dyn SpawnedChild>>>) -> Option<i32> {
    loop {
        let polled = {
            let mut guard = lock(child);
            let running = guard.as_mut()?;
            let polled = running.try_wait();
            if !matches!(polled, Ok(None)) {
                guard.take();
            }
            polled
        };
        match polled {
            Ok(Some(code)) => return Some(code),
            Ok(None) => thread::sleep(CHILD_EXIT_POLL_INTERVAL),
            Err(_) => return None,
        }
    }
}

/// The control response line allowing or denying the tool request `id`.
fn permission_response_line(id: &str, allow: bool, input: Value) -> ControlResponseLine {
    let (behavior, message) = if allow {
        (PermissionBehavior::Allow, None)
    } else {
        (PermissionBehavior::Deny, Some(DENY_MESSAGE.to_string()))
    };
    ControlResponseLine {
        message_type: StreamMessageType::ControlResponse,
        response: ControlResponseBody {
            subtype: ControlSubtype::Success,
            request_id: id.to_string(),
            response: PermissionDecision {
                behavior,
                message,
                updated_input: allow.then_some(input),
            },
        },
    }
}

fn exit_reason(code: Option<i32>, cancelled: bool) -> ExitReason {
    if cancelled {
        ExitReason::Cancelled
    } else if code == Some(0) {
        ExitReason::Completed
    } else {
        ExitReason::Failed
    }
}

fn content_blocks(message: &Value) -> Vec<&Value> {
    message
        .get(FIELD_MESSAGE)
        .and_then(|inner| inner.get(FIELD_CONTENT))
        .and_then(Value::as_array)
        .map(|blocks| blocks.iter().collect())
        .unwrap_or_default()
}

/// Reads `field` of `value` as the enum `T`, or `None` when absent or not a variant of `T`.
fn field_as<T: DeserializeOwned>(value: &Value, field: &str) -> Option<T> {
    serde_json::from_value(value.get(field)?.clone()).ok()
}

fn text_of(block: &Value, field: &str) -> String {
    block
        .get(field)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

fn token_count(usage: &Value, field: &str) -> u64 {
    usage.get(field).and_then(Value::as_u64).unwrap_or(0)
}

fn assistant_events(message: &Value) -> Vec<AgentEvent> {
    content_blocks(message)
        .into_iter()
        .filter_map(
            |block| match field_as::<ContentBlockType>(block, FIELD_TYPE)? {
                ContentBlockType::Text => Some(AgentEvent::Text {
                    text: text_of(block, FIELD_TEXT),
                }),
                ContentBlockType::ToolUse => Some(AgentEvent::ToolUse {
                    id: text_of(block, FIELD_ID),
                    name: text_of(block, FIELD_NAME),
                    input: block.get(FIELD_INPUT).cloned().unwrap_or(Value::Null),
                }),
                ContentBlockType::ToolResult => None,
            },
        )
        .collect()
}

fn user_events(message: &Value) -> Vec<AgentEvent> {
    content_blocks(message)
        .into_iter()
        .filter(|block| field_as(block, FIELD_TYPE) == Some(ContentBlockType::ToolResult))
        .map(|block| AgentEvent::ToolResult {
            id: text_of(block, FIELD_TOOL_USE_ID),
            output: block.get(FIELD_CONTENT).cloned().unwrap_or(Value::Null),
            is_error: block
                .get(FIELD_IS_ERROR)
                .and_then(Value::as_bool)
                .unwrap_or(false),
        })
        .collect()
}

fn result_events(message: &Value) -> Vec<AgentEvent> {
    message
        .get(FIELD_USAGE)
        .map(|usage| {
            AgentEvent::Usage(TokenUsage {
                input: token_count(usage, USAGE_INPUT),
                output: token_count(usage, USAGE_OUTPUT),
                cache_creation: token_count(usage, USAGE_CACHE_CREATION),
                cache_read: token_count(usage, USAGE_CACHE_READ),
            })
        })
        .into_iter()
        .collect()
}

fn control_request_events(message: &Value) -> Vec<AgentEvent> {
    let request = message.get(FIELD_REQUEST).unwrap_or(&Value::Null);
    if field_as(request, FIELD_SUBTYPE) != Some(ControlSubtype::CanUseTool) {
        return Vec::new();
    }
    vec![AgentEvent::PermissionRequest {
        id: text_of(message, FIELD_REQUEST_ID),
        tool: text_of(request, FIELD_TOOL_NAME),
        input: request.get(FIELD_INPUT).cloned().unwrap_or(Value::Null),
    }]
}

/// Normalized events of one stream-json message; empty for message types with no counterpart.
pub(crate) fn normalized_events(message: &Value) -> Vec<AgentEvent> {
    match field_as(message, FIELD_TYPE) {
        Some(StreamMessageType::Assistant) => assistant_events(message),
        Some(StreamMessageType::User) => user_events(message),
        Some(StreamMessageType::Result) => result_events(message),
        Some(StreamMessageType::ControlRequest) => control_request_events(message),
        _ => Vec::new(),
    }
}

/// Events of one stdout line: the raw message first, then its normalized form.
/// A line that is not JSON becomes a raw string.
fn events_for_line(line: &str) -> Vec<AgentEvent> {
    match serde_json::from_str::<Value>(line) {
        Ok(message) => {
            let mut events = vec![AgentEvent::Raw(message.clone())];
            events.extend(normalized_events(&message));
            events
        }
        Err(_) => vec![AgentEvent::Raw(Value::String(line.to_string()))],
    }
}

impl ExitSignal {
    fn new() -> Self {
        Self {
            reason: Mutex::new(None),
            ended: Condvar::new(),
        }
    }

    fn set(&self, reason: ExitReason) {
        *lock(&self.reason) = Some(reason);
        self.ended.notify_all();
    }

    fn wait(&self) -> ExitReason {
        let mut guard = lock(&self.reason);
        loop {
            if let Some(reason) = *guard {
                return reason;
            }
            guard = self
                .ended
                .wait(guard)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }
}

impl StdinGuard {
    /// Keeps the pipe open after this guard is dropped; the provider's controls still reach it.
    pub fn detach(mut self) {
        self.close_on_drop = false;
    }
}

impl Write for StdinGuard {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        match lock(&self.cell).as_mut() {
            Some(writer) => writer.write(buffer),
            None => Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                INPUT_CLOSED_MESSAGE,
            )),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match lock(&self.cell).as_mut() {
            Some(writer) => writer.flush(),
            None => Ok(()),
        }
    }
}

impl Drop for StdinGuard {
    fn drop(&mut self) {
        if self.close_on_drop {
            lock(&self.cell).take();
        }
    }
}

impl AgentControl for ClaudeControl {
    fn cancel(&self) -> Result<(), AgentError> {
        self.cancelled.store(true, Ordering::SeqCst);
        lock(&self.stdin).take();
        if let Some(child) = lock(&self.child).as_mut() {
            child.kill()?;
        }
        Ok(())
    }

    fn reply_permission(&self, id: &str, allow: bool) -> Result<(), AgentError> {
        let mut permissions = lock(&self.permissions);
        let input = permissions.get(id).cloned().ok_or_else(|| {
            AgentError::Config(
                crate::models::constants::claude_permissions::INVALID_REQUEST.to_string(),
            )
        })?;
        let line = serde_json::to_string(&permission_response_line(id, allow, input))
            .map_err(std::io::Error::other)?
            + "
";
        let mut guard = lock(&self.stdin);
        let writer = guard.as_mut().ok_or_else(|| {
            AgentError::Io(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                INPUT_CLOSED_MESSAGE,
            ))
        })?;
        writer.write_all(line.as_bytes())?;
        writer.flush()?;
        permissions.remove(id);
        Ok(())
    }

    fn wait(&self) -> Result<ExitReason, AgentError> {
        Ok(self.exit.wait())
    }
}

/// Reads stdout to its end, then reports the exit; send failures mean the consumer left.
fn pump(
    stdout: Box<dyn std::io::Read + Send>,
    events: Sender<AgentEvent>,
    child: ChildCell,
    cancelled: Arc<AtomicBool>,
    exit: Arc<ExitSignal>,
    permissions: PermissionCell,
    configuration: Option<Arc<aidp_agents::McpConfiguration>>,
) {
    for line in BufReader::new(stdout).lines().map_while(Result::ok) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        for event in events_for_line(trimmed) {
            if let AgentEvent::PermissionRequest { id, input, .. } = &event {
                lock(&permissions).insert(id.clone(), input.clone());
            }
            let _ = events.send(crate::mcp_provider::redact_event(
                configuration.as_deref(),
                event,
            ));
        }
    }
    let code = wait_for_child(&child);
    let reason = exit_reason(code, cancelled.load(Ordering::SeqCst));
    exit.set(reason);
    let _ = events.send(AgentEvent::Exited { code, reason });
}

impl ClaudeCodeProvider {
    pub fn new(runner: Arc<dyn ProcessRunner>) -> Self {
        Self { runner }
    }

    /// Starts the session and hands back its stdin so the caller sends the prompts itself.
    pub fn start_with_stdin(&self, spec: AgentSpec) -> Result<ClaudeLaunch, AgentError> {
        let mut args = claude_session_args(&spec)?;
        let mut environment = spec.env.clone();
        if let Some(configuration) = &spec.mcp_configuration {
            let (json, variables) = crate::mcp_provider::claude_configuration(configuration)?;
            args.extend([crate::models::constants::mcp::CONFIG_FLAG.to_string(), json]);
            environment.extend(variables);
        }
        let process = claude_process_spec(args, &spec.work_dir, &environment);
        let mut child = self.runner.spawn(&process).map_err(|error| {
            if spec.mcp_configuration.is_some() {
                AgentError::Spawn(crate::models::constants::mcp::CONFIGURATION_ERROR.to_string())
            } else {
                AgentError::Io(error)
            }
        })?;
        crate::mcp_provider::drain_stderr(child.as_mut(), spec.mcp_configuration.is_some());
        let stdout = child
            .take_stdout()
            .ok_or_else(|| AgentError::Spawn(NO_STDOUT_MESSAGE.to_string()))?;
        let stdin_cell: StdinCell = Arc::new(Mutex::new(child.take_stdin()));
        let has_stdin = lock(&stdin_cell).is_some();
        let child_cell: ChildCell = Arc::new(Mutex::new(Some(child)));
        let cancelled = Arc::new(AtomicBool::new(false));
        let exit = Arc::new(ExitSignal::new());
        let (sender, receiver) = mpsc::channel();
        let permissions: PermissionCell = Arc::new(Mutex::new(BTreeMap::new()));
        let pump_permissions = permissions.clone();
        let (pump_child, pump_cancelled, pump_exit) =
            (child_cell.clone(), cancelled.clone(), exit.clone());
        thread::spawn(move || {
            pump(
                stdout,
                sender,
                pump_child,
                pump_cancelled,
                pump_exit,
                pump_permissions,
                spec.mcp_configuration,
            )
        });
        let control = ClaudeControl {
            child: child_cell,
            stdin: stdin_cell.clone(),
            cancelled,
            exit,
            permissions,
        };
        Ok(ClaudeLaunch {
            handle: AgentHandle::new(receiver, Box::new(control)),
            stdin: has_stdin.then_some(StdinGuard {
                cell: stdin_cell,
                close_on_drop: true,
            }),
        })
    }
}

impl AgentProvider for ClaudeCodeProvider {
    fn id(&self) -> &str {
        CLAUDE_PROVIDER_ID
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            native_skills: true,
            hooks: true,
            permission_host: true,
            structured_output: true,
            resume: true,
            cost_reporting: true,
        }
    }

    /// Starts the session and sends `spec.prompt` as its first user message.
    fn start(&self, spec: AgentSpec) -> Result<AgentHandle, AgentError> {
        let prompt = spec.prompt.clone();
        let launch = self.start_with_stdin(spec)?;
        if let Some(mut stdin) = launch.stdin {
            if !prompt.is_empty() {
                stdin.write_all(user_message_line(&prompt).as_bytes())?;
                stdin.flush()?;
            }
            stdin.detach();
        }
        Ok(launch.handle)
    }
}

#[cfg(test)]
#[path = "../tests/unit/claude_argv_golden_tests.rs"]
mod claude_argv_golden_tests;

#[cfg(test)]
#[path = "../tests/unit/claude_permissions_tests.rs"]
mod claude_permissions_tests;
