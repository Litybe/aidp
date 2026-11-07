//! The GitHub verbs of `aidp ticket`: `sync`, `label` and `comment`. Reads and writes go through
//! the `gh` CLI ([`GhIssuePort`]); the ticket logic lives in `aidp_tickets::github_sync`.

use super::CommandOutput;
use aidp_integrations::github_issues::GhIssuePort;
use aidp_tickets::github_sync::{self, SyncOptions};
use aidp_tickets::models::github_port::GithubPort;
use aidp_tickets::store::TicketStore;
use std::io::Read;

const STDIN_MARKER: &str = "-";
const NO_BODY: &str = "`aidp ticket comment` needs --body-file or --body";
const NO_REPOS: &str =
    "no GitHub repository configured: set `github.repos` in <vault>/TODO/config.yaml or pass --repo";

/// Everything `aidp ticket sync` accepts.
pub struct SyncArgs {
    pub repo: Option<String>,
    pub limit: Option<u32>,
    pub dry_run: bool,
}

/// `aidp ticket sync --source github`: prints the report.
pub fn sync(store: &TicketStore, args: SyncArgs) -> anyhow::Result<CommandOutput> {
    if args.repo.is_none() && store.config().github.repos.is_empty() {
        anyhow::bail!(NO_REPOS);
    }
    let report = github_sync::sync(
        store,
        &GhIssuePort::new(),
        &SyncOptions {
            dry_run: args.dry_run,
            repo: args.repo,
            limit: args.limit,
        },
    )?;
    Ok(CommandOutput::text(serde_json::to_string(&report)?))
}

/// `aidp ticket label <key> <state>`.
pub fn label(store: &TicketStore, key: &str, state: &str) -> anyhow::Result<CommandOutput> {
    let outcome = github_sync::label_ticket(store, &GhIssuePort::new(), key, state)?;
    Ok(CommandOutput::text(serde_json::to_string(&outcome)?))
}

/// `aidp ticket comment <key> --body-file F | --body T [--cost-footer]`.
pub fn comment(
    store: &TicketStore,
    key: &str,
    body_file: Option<&str>,
    body: Option<&str>,
    cost_footer: bool,
) -> anyhow::Result<CommandOutput> {
    comment_with(
        store,
        &GhIssuePort::new(),
        key,
        body_file,
        body,
        cost_footer,
    )
}

/// [`comment`] over any port.
pub fn comment_with(
    store: &TicketStore,
    port: &dyn GithubPort,
    key: &str,
    body_file: Option<&str>,
    body: Option<&str>,
    cost_footer: bool,
) -> anyhow::Result<CommandOutput> {
    let mut text = match (body_file, body) {
        (Some(STDIN_MARKER), _) => {
            let mut text = String::new();
            std::io::stdin().read_to_string(&mut text)?;
            text
        }
        (Some(path), _) => std::fs::read_to_string(path)?,
        (None, Some(text)) => text.to_string(),
        (None, None) => anyhow::bail!(NO_BODY),
    };
    if cost_footer {
        if let Some(footer) = crate::cost_footer::cost_footer_for_ticket(key) {
            text = format!("{}\n\n{footer}\n", text.trim_end());
        }
    }
    let url = github_sync::comment_ticket(store, port, key, &text)?;
    Ok(CommandOutput::text(
        serde_json::json!({ "key": key, "url": url, "costFooter": cost_footer }).to_string(),
    ))
}
