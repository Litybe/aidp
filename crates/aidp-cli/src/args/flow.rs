//! The `aidp flow` subcommand tree.

use clap::Subcommand;
use std::path::PathBuf;

/// The `aidp flow` verbs. `validate`, `show` and the composite verbs run locally; the others call the daemon.
#[derive(Subcommand, Debug)]
pub enum Op {
    /// Validate a flow definition file against the pipeline templates and the built-in registry.
    Validate {
        /// Path to the flow YAML file.
        #[arg(value_name = "PATH")]
        path: PathBuf,
    },
    /// Print a flow definition file as parsed.
    Show {
        /// Path to the flow YAML file.
        #[arg(value_name = "PATH")]
        path: PathBuf,
    },
    /// Create the working branch of a ticket (`fix/` for a bug, `feat/` otherwise) when it is absent.
    GitBranch {
        #[arg(value_name = "KEY")]
        key: String,
    },
    /// Publish the worktree's branch as one commit GitHub signs, through its API: never `git push`.
    GitPush {
        /// Branch the worktree must be on; the current branch when omitted.
        #[arg(long)]
        branch: Option<String>,
    },
    /// Open the pull request of a ticket branch, or reuse the open one, and write its `pr.json`.
    PrCreate {
        #[arg(value_name = "KEY")]
        key: String,
        /// Head branch of the pull request; the current branch when omitted.
        #[arg(long)]
        branch: Option<String>,
        /// File holding the pull request body; `pr.json` is written next to it.
        #[arg(long = "body-file", value_name = "PATH")]
        body_file: PathBuf,
        /// Open the pull request as a draft.
        #[arg(long)]
        draft: bool,
    },
    /// List the flow runs known to the daemon.
    List {
        /// Override the daemon's base URL.
        #[arg(long)]
        url: Option<String>,
    },
    /// Start a flow for a ticket.
    Start {
        #[arg(value_name = "TICKET")]
        ticket: String,
        /// Flow id (default: the daemon's default flow).
        #[arg(long)]
        flow: Option<String>,
        /// Setting override as `key=value`; the value is JSON when it parses, else a string.
        #[arg(long = "setting", value_name = "KEY=VALUE")]
        settings: Vec<String>,
        /// Override the daemon's base URL.
        #[arg(long)]
        url: Option<String>,
    },
    /// Show one flow run.
    Status {
        #[arg(value_name = "KEY")]
        key: String,
        /// Override the daemon's base URL.
        #[arg(long)]
        url: Option<String>,
    },
    /// Approve a human step, or reject it with `--reject`.
    Approve {
        #[arg(value_name = "KEY")]
        key: String,
        #[arg(value_name = "STEP")]
        step: String,
        /// Reject the step instead of approving it.
        #[arg(long)]
        reject: bool,
        /// Override the daemon's base URL.
        #[arg(long)]
        url: Option<String>,
    },
    /// Report the verdict of a step that waits on one.
    Verdict {
        #[arg(value_name = "KEY")]
        key: String,
        #[arg(value_name = "STEP")]
        step: String,
        /// The verdict is positive. Without it the verdict is negative.
        #[arg(long)]
        ok: bool,
        /// Free-text report attached to the verdict.
        #[arg(long)]
        report: Option<String>,
        /// Override the daemon's base URL.
        #[arg(long)]
        url: Option<String>,
    },
    /// Retry a failed or escalated step.
    Retry {
        #[arg(value_name = "KEY")]
        key: String,
        #[arg(value_name = "STEP")]
        step: String,
        /// Override the daemon's base URL.
        #[arg(long)]
        url: Option<String>,
    },
    /// Skip a step.
    Skip {
        #[arg(value_name = "KEY")]
        key: String,
        #[arg(value_name = "STEP")]
        step: String,
        /// Override the daemon's base URL.
        #[arg(long)]
        url: Option<String>,
    },
    /// Pause a run once its current step ends.
    Pause {
        #[arg(value_name = "KEY")]
        key: String,
        /// Override the daemon's base URL.
        #[arg(long)]
        url: Option<String>,
    },
    /// Resume a paused run.
    Resume {
        #[arg(value_name = "KEY")]
        key: String,
        /// Override the daemon's base URL.
        #[arg(long)]
        url: Option<String>,
    },
    /// Abort a run. The worktree and the PR are kept.
    Abort {
        #[arg(value_name = "KEY")]
        key: String,
        /// Override the daemon's base URL.
        #[arg(long)]
        url: Option<String>,
    },
}
