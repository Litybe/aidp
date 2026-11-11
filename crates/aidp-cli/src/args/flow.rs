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
    /// Put the main checkout of the project on the ticket branch (created from the base branch when absent). Reports `blocked` when the checkout is dirty.
    GitCheckout {
        #[arg(value_name = "KEY")]
        key: String,
    },
    /// Give the main checkout back to the base branch when it is clean; otherwise leave it and comment on the issue.
    GitRestore {
        #[arg(value_name = "KEY")]
        key: String,
    },
    /// Run one compile gate of the Unity editor open on the workspace (after `git-checkout`), through the playtest package's file bridge. Skipped without a Unity project or an open editor; `blocked` when the editor never answers.
    UnityRefresh {
        /// Seconds to wait for the compile result before reporting the editor blocked.
        #[arg(long, value_name = "SECONDS", default_value_t = crate::models::constants::flow_unity::DEFAULT_TIMEOUT_SECONDS)]
        timeout: u64,
        /// Times the wait and the heartbeat tolerance are stretched; the doctor sets it for one run.
        #[arg(long = "wait-multiplier", value_name = "N", default_value_t = 1)]
        wait_multiplier: u32,
    },
    /// Publish the worktree's branch as one commit GitHub signs, through its API: never `git push`.
    GitPush {
        /// Branch the worktree must be on; the current branch when omitted.
        #[arg(long)]
        branch: Option<String>,
    },
    /// Raise the bug issue of a failed test under the ticket `KEY`: label `bug`, sub-issue of the ticket, branch base `feat/<KEY>`. Reports `capped: true` (and creates nothing) when the ticket already holds 3 bugs.
    BugCreate {
        #[arg(value_name = "KEY")]
        key: String,
        /// File holding the failure report.
        #[arg(long = "report-file", value_name = "PATH")]
        report_file: Option<PathBuf>,
        /// File holding the body the `bug-write` stage wrote. Missing, unreadable or empty: the body is the report alone.
        #[arg(long = "body-file", value_name = "PATH")]
        body_file: Option<PathBuf>,
        /// One-line summary of the failure (title of the bug).
        #[arg(long)]
        summary: Option<String>,
        /// Allow one bug past the cap of 3: the doctor proved a cause the earlier bugs did not fix.
        #[arg(long = "allow-extra")]
        allow_extra: bool,
    },
    /// Re-arm the bug cap of the ticket `KEY`: its closed bugs stop counting. Once per 24 h per ticket; reports `rearmed: false` inside that window.
    BugRearm {
        #[arg(value_name = "KEY")]
        key: String,
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
