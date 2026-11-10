//! `aidp flow bug-create`: the bug issue a failed test raises under the ticket it tested.

use super::{branch_of, report_output};
use crate::commands::CommandOutput;
use crate::ctx::Ctx;
use aidp_core::{ExitStatus, TypedExit};
use aidp_integrations::github_issues::GhIssuePort;
use aidp_tickets::models::constants::github::EXTRA_BUGS_AFTER_CAP;
use aidp_tickets::models::github_port::GithubPort;
use aidp_tickets::store::TicketStore;
use std::path::Path;

const NO_VAULT_MESSAGE: &str = "no vault: cannot read the ticket";

/// Raises the bug issue of ticket `key`, whose failure report is in `report_file`.
pub(super) fn bug_create(
    ctx: &Ctx,
    key: &str,
    report_file: Option<&Path>,
    body_file: Option<&Path>,
    summary: &str,
    allow_extra: bool,
) -> anyhow::Result<CommandOutput> {
    let vault =
        aidp_core::vault::resolve(ctx.vault_flag(), &ctx.workspace()).ok_or_else(|| TypedExit {
            status: ExitStatus::NotFound,
            message: NO_VAULT_MESSAGE.to_string(),
        })?;
    let store = TicketStore::open(&vault.root)?;
    create_with(
        &store,
        &GhIssuePort::new(),
        key,
        report_file,
        body_file,
        summary,
        allow_extra,
    )
}

/// [`bug_create`] over any store and port. The base branch of the bug is the branch of ticket `key`.
pub(crate) fn create_with(
    store: &TicketStore,
    port: &dyn GithubPort,
    key: &str,
    report_file: Option<&Path>,
    body_file: Option<&Path>,
    summary: &str,
    allow_extra: bool,
) -> anyhow::Result<CommandOutput> {
    let (ticket, _, _) = store.get(key)?;
    let base = branch_of(&ticket.key, ticket.r#type);
    let report = match report_file {
        Some(path) => std::fs::read_to_string(path).map_err(|error| TypedExit {
            status: ExitStatus::Usage,
            message: format!("report file {}: {error}", path.display()),
        })?,
        None => String::new(),
    };
    // The written body is optional: a missing or empty file is the fallback, never an error.
    let written = body_file
        .and_then(|path| std::fs::read_to_string(path).ok())
        .unwrap_or_default();
    let extra_bugs = if allow_extra { EXTRA_BUGS_AFTER_CAP } else { 0 };
    let outcome = aidp_tickets::bug_ticket::create_allowing(
        store, port, key, &base, summary, &written, &report, extra_bugs,
    )?;
    report_output(&outcome)
}

/// Re-arms the bug cap of ticket `key`: its closed bugs stop counting, once per guard window.
pub(super) fn bug_rearm(ctx: &Ctx, key: &str) -> anyhow::Result<CommandOutput> {
    let vault =
        aidp_core::vault::resolve(ctx.vault_flag(), &ctx.workspace()).ok_or_else(|| TypedExit {
            status: ExitStatus::NotFound,
            message: NO_VAULT_MESSAGE.to_string(),
        })?;
    let store = TicketStore::open(&vault.root)?;
    rearm_with(&store, &GhIssuePort::new(), key)
}

/// [`bug_rearm`] over any store and port.
pub(crate) fn rearm_with(
    store: &TicketStore,
    port: &dyn GithubPort,
    key: &str,
) -> anyhow::Result<CommandOutput> {
    let outcome = aidp_tickets::bug_ticket::rearm(store, port, key, chrono::Utc::now())?;
    report_output(&outcome)
}
