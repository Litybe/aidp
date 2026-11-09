//! `aidp flow bug-create`: the bug issue a failed test raises under the ticket it tested.

use super::{branch_of, report_output};
use crate::commands::CommandOutput;
use crate::ctx::Ctx;
use aidp_core::{ExitStatus, TypedExit};
use aidp_integrations::github_issues::GhIssuePort;
use aidp_tickets::models::github_port::GithubPort;
use aidp_tickets::store::TicketStore;
use std::path::Path;

const NO_VAULT_MESSAGE: &str = "no vault: cannot read the ticket";

/// Raises the bug issue of ticket `key`, whose failure report is in `report_file`.
pub(super) fn bug_create(
    ctx: &Ctx,
    key: &str,
    report_file: Option<&Path>,
    summary: &str,
) -> anyhow::Result<CommandOutput> {
    let vault =
        aidp_core::vault::resolve(ctx.vault_flag(), &ctx.workspace()).ok_or_else(|| TypedExit {
            status: ExitStatus::NotFound,
            message: NO_VAULT_MESSAGE.to_string(),
        })?;
    let store = TicketStore::open(&vault.root)?;
    create_with(&store, &GhIssuePort::new(), key, report_file, summary)
}

/// [`bug_create`] over any store and port. The base branch of the bug is the branch of ticket `key`.
pub(crate) fn create_with(
    store: &TicketStore,
    port: &dyn GithubPort,
    key: &str,
    report_file: Option<&Path>,
    summary: &str,
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
    let outcome = aidp_tickets::bug_ticket::create(store, port, key, &base, summary, &report)?;
    report_output(&outcome)
}
