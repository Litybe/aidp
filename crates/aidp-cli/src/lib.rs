//! aidp — single multi-command binary for ai-dev-pipeline.
//! Composition root: parse, build the context, dispatch, print, exit.

use aidp_core::ExitStatus;
use aidp_gateway::models::constants::launch_policy;
use clap::{CommandFactory, FromArgMatches};
use models::constants::embedded_command::{
    ERROR_PREFIX, LINE_ENDING, PROGRAM, SERVER_REFUSAL, THREAD_FAILURE,
};
pub use models::embedded_command_result::EmbeddedCommandResult;

mod args;
mod commands;
mod cost_footer;
mod ctx;
mod exit;
mod logging;
mod models;
mod output;
mod pipelines;

const LOG_TARGET: &str = "aidp_cli";
const COMMAND_STARTED: &str = "command started";

use args::gateway::Op as GatewayOp;
use args::{Cli, Cmd, TicketLoc};
use commands::CommandOutput;
use ctx::{Ctx, GlobalFlags};
use std::path::{Path, PathBuf};

/// Stack of the command thread; the clap tree and the dispatch frames outgrow the 1 MiB Windows default in debug builds.
const COMMAND_STACK_BYTES: usize = 16 * 1024 * 1024;

/// Runs the traditional process entry point, including logging and stream emission.
pub fn run_binary() -> std::process::ExitCode {
    std::thread::Builder::new()
        .stack_size(COMMAND_STACK_BYTES)
        .spawn(run)
        .ok()
        .and_then(|handle| handle.join().ok())
        .unwrap_or(ExitStatus::Internal.into())
}

/// Executes arguments excluding argv[0], without global logging initialization or stream emission.
pub fn execute_embedded(arguments: Vec<String>) -> EmbeddedCommandResult {
    std::thread::Builder::new()
        .stack_size(COMMAND_STACK_BYTES)
        .spawn(move || run_embedded(arguments))
        .ok()
        .and_then(|handle| handle.join().ok())
        .unwrap_or_else(|| command_error(ExitStatus::Internal, THREAD_FAILURE))
}

fn run_embedded(arguments: Vec<String>) -> EmbeddedCommandResult {
    let arguments = std::iter::once(PROGRAM.to_string()).chain(arguments);
    let (cli, _) = match parse_cli_from(arguments) {
        Ok(parsed) => parsed,
        Err(error) => {
            let is_stderr = error.use_stderr();
            let exit_code = if is_stderr {
                ExitStatus::Usage
            } else {
                ExitStatus::Success
            };
            return EmbeddedCommandResult {
                exit_code: i32::from(exit_code.code()),
                stdout: if is_stderr {
                    String::new()
                } else {
                    error.to_string()
                },
                stderr: if is_stderr {
                    error.to_string()
                } else {
                    String::new()
                },
            };
        }
    };
    let (globals, command) = split_globals(cli);
    if matches!(
        command,
        Cmd::Gateway {
            op: GatewayOp::Serve { .. }
        } | Cmd::Mock { .. }
    ) {
        return command_error(ExitStatus::PolicyRefusal, SERVER_REFUSAL);
    }
    match Ctx::new(globals).and_then(|context| execute(&context, command)) {
        Ok(output) => finish_embedded(output),
        Err(error) => command_error(exit::classify(&error), &format!("{error:?}")),
    }
}

fn finish_embedded(output: CommandOutput) -> EmbeddedCommandResult {
    match output::capture(&output) {
        Ok((stdout, stderr)) => EmbeddedCommandResult {
            exit_code: output.child_exit.map_or_else(
                || i32::from(exit::exit_status(&output).code()),
                |code| i32::from(u8::try_from(code).unwrap_or(u8::MAX)),
            ),
            stdout,
            stderr,
        },
        Err(error) => command_error(exit::classify(&error), &format!("{error:?}")),
    }
}

fn command_error(status: ExitStatus, message: &str) -> EmbeddedCommandResult {
    EmbeddedCommandResult {
        exit_code: i32::from(status.code()),
        stdout: String::new(),
        stderr: format!("{ERROR_PREFIX}{message}{LINE_ENDING}"),
    }
}

/// Parses, dispatches and prints one command.
fn run() -> std::process::ExitCode {
    let (cli, command) = match parse_cli() {
        Ok(parsed) => parsed,
        Err(e) => {
            use clap::error::ErrorKind as ClapErrorKind;
            e.print().ok();
            return match e.kind() {
                ClapErrorKind::DisplayHelp | ClapErrorKind::DisplayVersion => {
                    std::process::ExitCode::SUCCESS
                }
                _ => ExitStatus::Usage.into(),
            };
        }
    };
    if std::env::var_os(launch_policy::HEADLESS_SESSION_ENV).is_some()
        && headless_session_refuses(&command)
    {
        eprintln!("Error: {}", launch_policy::HEADLESS_SESSION_REFUSAL);
        return ExitStatus::PolicyRefusal.into();
    }
    let (globals, cmd) = split_globals(cli);
    let _log_guard = logging::init_logging(globals.verbose, gateway_log_dir(&cmd).as_deref());
    tracing::info!(target: LOG_TARGET, command = command.as_str(), "{}", COMMAND_STARTED);
    match Ctx::new(globals).and_then(|ctx| execute(&ctx, cmd)) {
        Ok(output) => finish(output),
        Err(e) => {
            eprintln!("Error: {e:?}");
            exit::classify(&e).into()
        }
    }
}

/// The state directory receiving the gateway log file, only for `gateway serve`.
fn gateway_log_dir(cmd: &Cmd) -> Option<PathBuf> {
    match cmd {
        Cmd::Gateway {
            op: GatewayOp::Serve { state_dir, .. },
        } => state_dir
            .clone()
            .or_else(|| aidp_gateway::paths::state_dir().ok()),
        _ => None,
    }
}

/// Parses the command line, returning the parsed CLI with its subcommand path, such as `gh pr-merge`.
fn parse_cli() -> Result<(Cli, String), clap::Error> {
    parse_cli_from(std::env::args_os())
}

/// True when `command`, a subcommand path, is a verb that only the flow may run, never a headless session.
fn headless_session_refuses(command: &str) -> bool {
    let invoked = format!("{PROGRAM} {command} ");
    launch_policy::HEADLESS_DENIED_COMMANDS
        .iter()
        .any(|prefix| invoked.starts_with(&format!("{prefix} ")))
}

fn parse_cli_from<I, T>(arguments: I) -> Result<(Cli, String), clap::Error>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone,
{
    let matches = Cli::command().try_get_matches_from(arguments)?;
    let mut words = Vec::new();
    let mut current = &matches;
    while let Some((name, nested)) = current.subcommand() {
        words.push(name);
        current = nested;
    }
    let command = words.join(" ");
    Cli::from_arg_matches(&matches).map(|cli| (cli, command))
}

/// Prints what the command produced, then maps its outcome to the process exit code.
fn finish(output: CommandOutput) -> std::process::ExitCode {
    if let Err(e) = output::emit(&output) {
        eprintln!("Error: {e:?}");
        return exit::classify(&e).into();
    }
    match output.child_exit {
        Some(code) => child_exit_code(code),
        None => exit::exit_status(&output).into(),
    }
}

/// A child's own exit code as a process exit code, saturating above `u8::MAX`.
fn child_exit_code(code: i32) -> std::process::ExitCode {
    std::process::ExitCode::from(u8::try_from(code).unwrap_or(u8::MAX))
}

/// Separates the flags valid on every command from the command itself.
fn split_globals(cli: Cli) -> (GlobalFlags, Cmd) {
    let globals = GlobalFlags {
        workspace_root: cli.workspace_root,
        vault: cli.vault,
        no_vault: cli.no_vault,
        footer: cli.format == Some(crate::models::FormatArg::Footer),
        format: cli.format.map(Into::into),
        verbose: cli.verbose,
    };
    (globals, cli.cmd)
}

fn execute(ctx: &Ctx, cmd: Cmd) -> anyhow::Result<CommandOutput> {
    tracing::debug!(target: LOG_TARGET, "migrating legacy state");
    migrate_legacy_state(ctx);
    tracing::trace!(target: LOG_TARGET, "dispatching command");
    dispatch(ctx, cmd)
}

/// One-time legacy-state migration, best-effort: a repo's `.aidp/` marker files
/// move into the resolved vault before any command reads or writes them.
fn migrate_legacy_state(ctx: &Ctx) {
    let workspace = ctx.workspace();
    let vault = ctx.vault_override();
    let _ = aidp_core::vault::migrate_repo_state(vault.as_ref(), &workspace);
}

/// The ticket vault location carried by the global `--vault` and `--workspace-root`.
fn ticket_loc(ctx: &Ctx) -> TicketLoc {
    TicketLoc {
        vault: ctx.vault_flag().map(Path::to_path_buf),
        workspace_root: ctx.workspace_root_flag().map(Path::to_path_buf),
    }
}

fn dispatch(ctx: &Ctx, cmd: Cmd) -> anyhow::Result<CommandOutput> {
    match cmd {
        Cmd::Validate {
            pipeline,
            all,
            inputs,
        } => commands::validate::run(
            ctx,
            commands::validate::ValidateArgs {
                pipeline,
                all,
                inputs,
                workspace_root: ctx.workspace_root_flag().map(Path::to_path_buf),
            },
        ),
        Cmd::Pipeline { op } => commands::pipeline::run(ctx, op),
        Cmd::Run { op } => commands::run::run(ctx, op),
        Cmd::Scope { op } => commands::scope::run(ctx, op),
        Cmd::Agents { op } => commands::agents::run(ctx, op),
        Cmd::Skills { op } => commands::skills::run(ctx, op),
        Cmd::SkillRules { op } => commands::skill_rules::run(ctx, op),
        Cmd::Topo { pipeline } => commands::topo::run(ctx, commands::topo::TopoArgs { pipeline }),
        Cmd::Progress { run } => commands::progress::run(
            ctx,
            commands::progress::ProgressArgs {
                run,
                workspace_root: ctx.workspace_root_flag().map(Path::to_path_buf),
            },
        ),
        Cmd::Gate { op } => commands::gate::run(ctx, op),
        Cmd::Gh { op } => commands::gh::run(ctx, op),
        Cmd::Project { op } => commands::project::run(ctx, op),
        Cmd::Mock { addr, data } => {
            commands::mock::run(ctx, commands::mock::MockArgs { addr, data })
        }
        Cmd::Workflow { op } => commands::workflow::run(ctx, op),
        Cmd::Vault { op } => commands::vault::run(ctx, op),
        Cmd::Knowledge { op } => commands::knowledge::run(ctx, op),
        Cmd::Gateway { op } => commands::gateway::run(ctx, op),
        Cmd::Flow { op } => commands::flow::run(ctx, op),
        Cmd::Rag { op } => commands::memory::run_rag(ctx, op),
        Cmd::Memory { op } => commands::memory::run_memory(ctx, op),
        Cmd::Ticket { op } => commands::ticket::run(ctx, ticket_loc(ctx), op),
        Cmd::Doctor => commands::doctor::run(
            ctx,
            commands::doctor::DoctorArgs {
                format: ctx.format(),
                workspace_root: ctx.workspace_root_flag().map(Path::to_path_buf),
            },
        ),
        Cmd::Status => commands::status::run(
            ctx,
            commands::status::StatusArgs {
                format: ctx.format(),
                workspace_root: ctx.workspace_root_flag().map(Path::to_path_buf),
            },
        ),
        Cmd::Setup { op } => commands::setup::run(ctx, op),
        Cmd::Cost { op } => commands::cost::run(ctx, op),
        Cmd::Git { op } => commands::git::run(ctx, op),
        Cmd::Findings { op } => commands::findings::run(ctx, op),
    }
}

#[cfg(test)]
#[path = "../tests/unit/embedded_command_tests.rs"]
mod embedded_command_tests;

#[cfg(test)]
#[path = "../tests/unit/flow_action_contract_tests.rs"]
mod flow_action_contract_tests;

#[cfg(test)]
#[path = "../tests/unit/headless_session_guard_tests.rs"]
mod headless_session_guard_tests;
