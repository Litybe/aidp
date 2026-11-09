//! The `aidp ticket` subcommand tree.

use crate::models::{IssueTypeArg, PriorityArg, SourceArg, StatusArg, TrackArg};
use aidp_tickets::models::constants::cli::{
    HELP_BUDGET, HELP_DISPATCH, HELP_EDIT, HELP_LIST_REASON, HELP_NEW, HELP_PLAN, HELP_REINDEX,
    HELP_RENDER, HELP_ROUTE, HELP_ROUTE_FROM_UNROUTED, HELP_START, HELP_STATUS,
};
use clap::Subcommand;
use std::path::PathBuf;

/// The `aidp ticket` verbs.
#[derive(Subcommand, Debug)]
pub enum Op {
    /// Create a new local ticket; body precedence `--body-file` > `--body` > `--stdin` > template.
    /// Prints `{key, path, warnings}`.
    #[command(about = HELP_NEW)]
    New {
        #[arg(long)]
        service: String,
        #[arg(long)]
        summary: String,
        /// Ticket type (default: task).
        #[arg(long = "type", value_enum, ignore_case = true)]
        issue_type: Option<IssueTypeArg>,
        /// Priority name, P1..P5, or 1..5 (default: medium).
        #[arg(long, value_enum, ignore_case = true)]
        priority: Option<PriorityArg>,
        #[arg(long = "body-file")]
        body_file: Option<PathBuf>,
        #[arg(long)]
        body: Option<String>,
        /// Read the body from stdin.
        #[arg(long)]
        stdin: bool,
        /// Comma-separated, e.g. `a,b`.
        #[arg(long)]
        labels: Option<String>,
        #[arg(long)]
        template: Option<String>,
        /// Pipeline depth.
        #[arg(long, value_enum)]
        track: Option<TrackArg>,
        #[arg(long)]
        epic: Option<String>,
        /// Bypass `store::check_service`'s "known service" check.
        #[arg(long = "allow-unknown-service")]
        allow_unknown_service: bool,
    },
    /// List tickets in dequeue order.
    List {
        /// Comma-separated status names, e.g. `todo,blocked`.
        #[arg(long, value_enum, ignore_case = true, value_delimiter = ',')]
        status: Vec<StatusArg>,
        #[arg(long)]
        service: Option<String>,
        /// Ticket origin.
        #[arg(long, value_enum)]
        source: Option<SourceArg>,
        #[arg(long)]
        archived: bool,
        /// Keep only tickets whose `autopilot.blocked_reason` equals this value.
        #[arg(long, help = HELP_LIST_REASON)]
        reason: Option<String>,
    },
    /// Show one ticket: frontmatter + body (json = `Ticket` + body).
    Show {
        #[arg(value_name = "KEY")]
        key: String,
    },
    /// Print a ticket's body exactly as passed as the `prompt` input.
    /// No frontmatter, no added heading.
    #[command(about = HELP_RENDER)]
    Render {
        #[arg(value_name = "KEY")]
        key: String,
    },
    /// Edit a ticket's frontmatter, never the body.
    /// `--service` delegates to `route`.
    #[command(about = HELP_EDIT)]
    Edit {
        #[arg(value_name = "KEY")]
        key: String,
        #[arg(long, value_enum, ignore_case = true)]
        priority: Option<PriorityArg>,
        #[arg(long)]
        service: Option<String>,
        #[arg(long)]
        template: Option<String>,
        #[arg(long, value_enum)]
        track: Option<TrackArg>,
        #[arg(long)]
        summary: Option<String>,
        #[arg(long)]
        labels: Option<String>,
        #[arg(long)]
        epic: Option<String>,
    },
    /// Manual status transition, actor=human, journalled.
    /// Refused with a non-zero exit if illegal.
    #[command(about = HELP_STATUS)]
    Status {
        #[arg(value_name = "KEY")]
        key: String,
        #[arg(value_name = "STATUS", value_enum)]
        status: StatusArg,
        #[arg(long)]
        reason: Option<String>,
        /// Required to leave `in-progress` as a human-initiated transition.
        #[arg(long)]
        force: bool,
    },
    /// Move a ticket to another service, or re-route the `unrouted/` backlog with `--from-unrouted`.
    /// Exactly one of "KEY and --service" or `--from-unrouted` is required.
    #[command(about = HELP_ROUTE)]
    Route {
        #[arg(
            value_name = "KEY",
            conflicts_with = "from_unrouted",
            requires = "service"
        )]
        key: Option<String>,
        #[arg(long, conflicts_with = "from_unrouted")]
        service: Option<String>,
        #[arg(long)]
        template: Option<String>,
        /// Re-route every ticket parked under `unrouted` against the current rules.
        /// Mutually exclusive with KEY/`--service`.
        #[arg(long = "from-unrouted", help = HELP_ROUTE_FROM_UNROUTED, required_unless_present = "key")]
        from_unrouted: bool,
    },
    /// `status <KEY> cancelled`, actor=human, default reason `cancelled-by-user`.
    Cancel {
        #[arg(value_name = "KEY")]
        key: String,
        #[arg(long)]
        reason: Option<String>,
    },
    /// Read the transition journal, most recent last.
    Log {
        #[arg(value_name = "KEY")]
        key: Option<String>,
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
    /// Regenerate `_TODO.md` and every `_<service>.md`, archiving old done tickets first.
    #[command(about = HELP_REINDEX)]
    Reindex,
    /// Start a pipeline run for one ticket now, bypassing the scheduler.
    /// Declared here for help and parsing only; `aidp-cli` serves it through the gateway client.
    #[command(about = HELP_START)]
    Start {
        #[arg(value_name = "KEY")]
        key: String,
        /// Start even if the ticket's project already looks busy.
        #[arg(long)]
        force: bool,
        #[arg(long)]
        service: Option<String>,
        #[arg(long)]
        template: Option<String>,
        /// Pipeline depth.
        #[arg(long, value_enum)]
        track: Option<TrackArg>,
        /// Override the daemon's base URL (default: the configured
        /// gateway).
        #[arg(long)]
        url: Option<String>,
    },
    /// Adopt an existing PR for a ticket (actor human).
    /// Declared here for help and parsing only; `aidp-cli` serves it through the gateway client.
    Adopt {
        #[arg(value_name = "KEY")]
        key: String,
        /// The PR to adopt: a number or a URL.
        #[arg(long)]
        pr: String,
        /// Override the daemon's base URL (default: the configured
        /// gateway).
        #[arg(long)]
        url: Option<String>,
    },
    /// Mirror the open issues of the configured GitHub repositories as tickets
    /// (`TODO/<repo>/GH-<n>.md`), then write each mirror's state label back.
    /// Prints the sync report.
    Sync {
        /// Ticket source to pull; only `github` exists.
        #[arg(long, default_value = "github", value_parser = ["github"])]
        source: String,
        /// Only this repository (`owner/name`).
        #[arg(long)]
        repo: Option<String>,
        /// Open issues read per repository (default: `github.limit`).
        #[arg(long)]
        limit: Option<u32>,
        /// Compute the report, write nothing (no note, no label).
        #[arg(long)]
        dry_run: bool,
    },
    /// Replace the single `aidp:*` state label of a GitHub mirror's issue.
    /// `STATE` is a suffix such as `review` or a full label such as `aidp:review`.
    Label {
        #[arg(value_name = "KEY")]
        key: String,
        #[arg(value_name = "STATE")]
        state: String,
    },
    /// Close the issue of a GitHub mirror (a bug merged into its parent branch is not closed by GitHub itself).
    Close {
        #[arg(value_name = "KEY")]
        key: String,
    },
    /// Post a comment on the issue of a GitHub mirror.
    Comment {
        #[arg(value_name = "KEY")]
        key: String,
        /// File holding the comment; `-` reads standard input.
        #[arg(long = "body-file", conflicts_with = "body")]
        body_file: Option<String>,
        /// The comment text.
        #[arg(long)]
        body: Option<String>,
        /// Append the ticket's token cost line to the comment.
        #[arg(long = "cost-footer")]
        cost_footer: bool,
    },
    /// Local preview of [`plan::plan`] with no daemon, nothing running and an unconstrained budget.
    /// For the daemon-aware answer use `aidp gateway dispatcher status`.
    #[command(about = HELP_PLAN)]
    Plan,
    /// Ask the running daemon to run one dispatch tick now.
    /// Declared for help and parsing only; `aidp-cli` serves it. For a local preview use [`Op::Plan`].
    #[command(about = HELP_DISPATCH)]
    Dispatch {
        /// Compute the tick's plan without actually launching anything.
        #[arg(long)]
        dry_run: bool,
        /// Override the daemon's base URL (default: the configured
        /// gateway).
        #[arg(long)]
        url: Option<String>,
    },
    /// Explain the token-budget verdict the daemon computed on its last tick.
    /// Declared for help and parsing only; `aidp-cli` serves it through the gateway client.
    #[command(about = HELP_BUDGET)]
    Budget {
        /// Override the daemon's base URL (default: the configured gateway).
        #[arg(long)]
        url: Option<String>,
    },
}
