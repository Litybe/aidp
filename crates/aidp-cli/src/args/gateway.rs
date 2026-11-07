//! The `aidp gateway` subcommand tree.

use crate::models::enums::engine_arg::EngineArg;
use aidp_gateway::models::enums::WorkspaceKind;
use aidp_gateway::models::WorkspaceRequest;
use clap::Subcommand;
use std::net::IpAddr;
use std::path::PathBuf;
use std::str::FromStr;

const SPEC_SEPARATOR: char = ':';
const SPEC_HELP: &str = "project | ticket:<KEY> | scratch:<name> | path:<dir>";

fn workspace_request(
    kind: WorkspaceKind,
    ticket: Option<&str>,
    name: Option<&str>,
    path: Option<&str>,
) -> WorkspaceRequest {
    WorkspaceRequest {
        kind,
        ticket: ticket.map(str::to_string),
        name: name.map(str::to_string),
        path: path.map(str::to_string),
    }
}

/// Boxed variant of [`parse_workspace_spec`] so the clap enum stays small.
pub fn parse_boxed_workspace_spec(spec: &str) -> Result<Box<WorkspaceRequest>, String> {
    parse_workspace_spec(spec).map(Box::new)
}

/// Parses `--workspace` into a request; the value is mandatory for every kind but `project`.
pub fn parse_workspace_spec(spec: &str) -> Result<WorkspaceRequest, String> {
    let (kind, value) = match spec.split_once(SPEC_SEPARATOR) {
        Some((kind, value)) => (kind, Some(value.trim())),
        None => (spec, None),
    };
    let value = value.filter(|v| !v.is_empty());
    let invalid = || format!("invalid workspace {spec:?}, expected {SPEC_HELP}");
    let kind = WorkspaceKind::from_str(kind).map_err(|_| invalid())?;
    match (kind, value) {
        (WorkspaceKind::Project, None) => Ok(workspace_request(kind, None, None, None)),
        (WorkspaceKind::Ticket, Some(key)) => Ok(workspace_request(kind, Some(key), None, None)),
        (WorkspaceKind::Scratch, Some(name)) => Ok(workspace_request(kind, None, Some(name), None)),
        (WorkspaceKind::Path, Some(dir)) => Ok(workspace_request(kind, None, None, Some(dir))),
        _ => Err(invalid()),
    }
}

#[derive(Subcommand, Debug)]
pub enum Op {
    /// Run the gateway daemon (sessions, automations, HTTP API + SSE).
    Serve {
        #[arg(long, default_value_t = 3001)]
        port: u16,
        #[arg(long, default_value = "127.0.0.1")]
        bind: IpAddr,
        /// State directory (default: $AIDP_GATEWAY_DIR or ~/.aidp/gateway).
        #[arg(long)]
        state_dir: Option<PathBuf>,
        /// Web UI build served on / with an index.html fallback, /api first (default: $AIDP_UI_DIR).
        #[arg(long)]
        ui_dir: Option<PathBuf>,
    },
    /// Daemon status: sessions, automations, rate-limit windows.
    Status {
        #[arg(long)]
        url: Option<String>,
    },
    /// List sessions.
    Sessions {
        #[arg(long)]
        url: Option<String>,
        #[arg(long)]
        status: Option<String>,
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
    /// Start a session: a pipeline (`--template`, `--target`) or a free prompt (`--prompt`).
    Start {
        #[arg(long)]
        url: Option<String>,
        /// Registered project id or name.
        #[arg(long)]
        project: String,
        #[arg(long)]
        template: Option<String>,
        /// native (default) | workflow: how a `--template` run executes.
        #[arg(long, value_enum)]
        engine: Option<EngineArg>,
        #[arg(long)]
        target: Option<String>,
        /// Repeatable `key=value` pipeline input.
        #[arg(long = "input")]
        inputs: Vec<String>,
        #[arg(long)]
        prompt: Option<String>,
        /// default | acceptEdits | bypassPermissions | plan
        #[arg(long)]
        permission_mode: Option<String>,
        #[arg(long)]
        model: Option<String>,
        #[arg(long)]
        effort: Option<String>,
        #[arg(long)]
        max_budget_usd: Option<f64>,
        /// Inject the artefacts of that session's run into the prompt (chains a Study into a Dev, etc).
        #[arg(long)]
        context_from: Option<String>,
        /// Keep the process open after the first turn (default: close it, this is an autonomous session).
        #[arg(long)]
        keep_open: bool,
        /// Where the session runs: project | ticket:<KEY> | scratch:<name> | path:<dir>.
        #[arg(long, value_parser = parse_boxed_workspace_spec)]
        workspace: Option<Box<WorkspaceRequest>>,
    },
    /// Send free text to a running session (gate answers go through the viewer).
    Reply {
        #[arg(long)]
        url: Option<String>,
        id: String,
        #[arg(long)]
        text: String,
    },
    /// Cancel a running session.
    Cancel {
        #[arg(long)]
        url: Option<String>,
        id: String,
    },
    /// Manage automations.
    Automations {
        #[arg(long)]
        url: Option<String>,
        #[command(subcommand)]
        op: AutoOp,
    },
    /// Manage the project registry the gateway launches sessions in.
    Projects {
        #[arg(long)]
        url: Option<String>,
        #[command(subcommand)]
        op: ProjOp,
    },
    /// Manage launch presets.
    Presets {
        #[arg(long)]
        url: Option<String>,
        #[command(subcommand)]
        op: PresetOp,
    },
    /// Inspect and steer the per-project launch queue.
    Queue {
        #[arg(long)]
        url: Option<String>,
        #[command(subcommand)]
        op: QueueOp,
    },
    /// Inspect and prune the per-ticket git worktrees.
    Worktrees {
        #[arg(long)]
        url: Option<String>,
        #[command(subcommand)]
        op: WorktreesOp,
    },
    /// Manage the ticket-autopilot dispatcher (lot 9a): the resident loop
    /// that turns `todo` tickets into running sessions.
    Dispatcher {
        #[arg(long)]
        url: Option<String>,
        #[command(subcommand)]
        op: DispatcherOp,
    },
    /// Recent gateway events.
    Events {
        #[arg(long)]
        url: Option<String>,
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
}

#[derive(Subcommand, Debug)]
pub enum AutoOp {
    List {},
    /// Add (or update, when the JSON carries an `id`) an automation from a JSON file.
    Add {
        file: PathBuf,
    },
    Rm {
        id: String,
    },
    /// Fire an automation now.
    Run {
        id: String,
    },
    Enable {
        id: String,
    },
    Disable {
        id: String,
    },
    /// Runs history (all automations, or one).
    Runs {
        #[arg(long)]
        id: Option<String>,
        #[arg(long, default_value_t = 30)]
        limit: usize,
    },
}

#[derive(Subcommand, Debug)]
pub enum ProjOp {
    List,
    /// Register a project by absolute path.
    Add {
        path: String,
    },
    Rm {
        id: String,
    },
    /// Import projects from an aidp-viewer `config/config.json` (`{"projects":[{path,...}]}`).
    Import {
        file: PathBuf,
    },
}

#[derive(Subcommand, Debug)]
pub enum PresetOp {
    List {},
    /// Add (or update, when the JSON carries an `id`) a preset from a JSON file.
    Add {
        file: PathBuf,
    },
    Rm {
        id: String,
    },
    /// Start a session from a preset.
    Start {
        id: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum QueueOp {
    /// List queued launches, in dispatch order.
    List {
        #[arg(long)]
        project_id: Option<String>,
    },
    /// Make a queued launch the next one out for its project.
    Promote { id: String },
    /// Drop a queued launch.
    Cancel { id: String },
}

#[derive(Subcommand, Debug)]
pub enum WorktreesOp {
    /// List the worktrees, optionally of one project.
    List {
        #[arg(long)]
        project: Option<String>,
    },
    /// Remove the prunable worktrees (done, cancelled or unknown tickets).
    Prune {
        /// Report the prunable worktrees without removing anything.
        #[arg(long)]
        dry_run: bool,
    },
    /// Create or reuse the worktree of a ticket, within the per-project cap.
    Ensure { key: String },
    /// Save the unsaved work of a ticket's worktree on its branch, then delete it.
    Remove { key: String },
}

#[derive(Subcommand, Debug)]
pub enum DispatcherOp {
    /// Current dispatcher state (mode, paused, slots, running, last plan) —
    /// the same shape persisted to `TODO/.state/dispatcher.json`.
    Status {},
    /// Pause dispatching (persisted to `TODO/config.yaml`'s `dispatcher.paused`).
    Pause,
    /// Resume dispatching (persisted to `TODO/config.yaml`'s `dispatcher.paused`).
    Resume,
    /// Run one dispatch cycle now.
    Tick {
        /// Build the plan without launching anything.
        #[arg(long)]
        dry_run: bool,
    },
}

#[cfg(test)]
#[path = "../../tests/unit/gateway_worktrees_args_tests.rs"]
mod gateway_worktrees_args_tests;
