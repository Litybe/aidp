//! `flow git-checkout` and `flow git-restore`: a flow with `workspace: project` works in the main
//! checkout of the project. The first puts it on the branch of the ticket (created from the base
//! branch when absent), the second gives it back to the base branch.

use super::{
    branch_of, flow_ticket, foreign_authors, git_output, git_stdout, git_succeeds, issue_type_of,
    typed, verify_args, CommandOutput, Ctx,
};
use crate::models::constants::flow_git::*;
use aidp_core::ExitStatus;
use serde_json::{json, Value};
use std::path::Path;

const GIT_STATUS: &str = "status";
const GIT_PORCELAIN: &str = "--porcelain";
const GIT_SHOW_CURRENT: [&str; 2] = ["branch", "--show-current"];
const GIT_ORIGIN_HEAD: [&str; 3] = ["symbolic-ref", "--short", "refs/remotes/origin/HEAD"];
const GIT_NO_TRACK: &str = "--no-track";
const BASE_CANDIDATES: [&str; 2] = ["main", "master"];
const FILES_SHOWN: usize = 20;
const BLOCKED_DIRTY: &str = "dirty-checkout";

/// The branch of the ticket: `fix/` for a bug, `feat/` otherwise, followed by the ticket key.
fn ticket_branch(ctx: &Ctx, key: &str) -> anyhow::Result<String> {
    let ticket_key = flow_ticket(ctx, key)
        .as_ref()
        .and_then(|ticket| ticket.get(aidp_gateway::models::constants::flow_execution::TICKET_KEY))
        .and_then(Value::as_str)
        .unwrap_or(key)
        .to_string();
    Ok(branch_of(&ticket_key, issue_type_of(ctx, key)?))
}

/// The base branch: the remote's default branch, else `main`, else `master`.
fn base_branch(ctx: &Ctx, workspace: &Path) -> anyhow::Result<String> {
    if let Ok(head) = git_stdout(ctx, workspace, &GIT_ORIGIN_HEAD) {
        if let Some(name) = head.trim().strip_prefix("origin/") {
            return Ok(name.to_string());
        }
    }
    for candidate in BASE_CANDIDATES {
        let local = format!("{LOCAL_REF_PREFIX}{candidate}");
        if git_succeeds(ctx, workspace, &verify_args(&local))? {
            return Ok(candidate.to_string());
        }
    }
    Err(typed(
        ExitStatus::NotFound,
        "no base branch found (origin/HEAD, main or master)",
    ))
}

fn current_branch(ctx: &Ctx, workspace: &Path) -> anyhow::Result<String> {
    Ok(git_stdout(ctx, workspace, &GIT_SHOW_CURRENT)?
        .trim()
        .to_string())
}

/// The paths `git status --porcelain` lists (untracked included).
fn dirty_files(ctx: &Ctx, workspace: &Path) -> anyhow::Result<Vec<String>> {
    Ok(git_stdout(ctx, workspace, &[GIT_STATUS, GIT_PORCELAIN])?
        .lines()
        .map(|line| line.trim_end().to_string())
        .filter(|line| !line.is_empty())
        .collect())
}

fn files_text(files: &[String]) -> String {
    let mut lines: Vec<String> = files
        .iter()
        .take(FILES_SHOWN)
        .map(|file| format!("- `{file}`"))
        .collect();
    if files.len() > FILES_SHOWN {
        lines.push(format!("- ... et {} autres", files.len() - FILES_SHOWN));
    }
    lines.join("\n")
}

fn output(value: Value) -> anyhow::Result<CommandOutput> {
    Ok(CommandOutput::text(value.to_string()))
}

fn switched(branch: &str, base: &str, created: bool) -> anyhow::Result<CommandOutput> {
    output(json!({ "branch": branch, "base": base, "created": created, "blocked": false }))
}

/// Puts the main checkout on the branch of the ticket. A dirty checkout on another branch is
/// reported as `blocked` and left untouched.
pub(super) fn git_checkout(ctx: &Ctx, key: &str) -> anyhow::Result<CommandOutput> {
    let workspace = ctx.workspace();
    let branch = ticket_branch(ctx, key)?;
    let base = base_branch(ctx, &workspace)?;
    let current = current_branch(ctx, &workspace)?;
    if current == branch {
        return switched(&branch, &base, false);
    }
    let files = dirty_files(ctx, &workspace)?;
    if !files.is_empty() {
        let message = format!(
            "blocked: {BLOCKED_DIRTY}. Le checkout principal a des modifications locales sur `{current}` :\n{}\nValide-les ou jette-les, puis relance.",
            files_text(&files)
        );
        return output(json!({
            "branch": branch, "base": base, "created": false, "blocked": true,
            "blocked_reason": BLOCKED_DIRTY, "message": message, "files": files,
        }));
    }
    let local_ref = format!("{LOCAL_REF_PREFIX}{branch}");
    if git_succeeds(ctx, &workspace, &verify_args(&local_ref))? {
        git_stdout(ctx, &workspace, &[GIT_SWITCH, &branch])?;
        return switched(&branch, &base, false);
    }
    let listed = git_stdout(
        ctx,
        &workspace,
        &[GIT_LS_REMOTE, GIT_HEADS_FLAG, ORIGIN_REMOTE, &branch],
    )?;
    if listed.trim().is_empty() {
        let _ = git_output(ctx, &workspace, &[GIT_FETCH, ORIGIN_REMOTE, &base]);
        let remote_base = format!("{ORIGIN_REMOTE}/{base}");
        let start = if git_succeeds(ctx, &workspace, &verify_args(&remote_base))? {
            remote_base
        } else {
            base.clone()
        };
        git_stdout(
            ctx,
            &workspace,
            &[GIT_SWITCH, GIT_CREATE_FLAG, &branch, GIT_NO_TRACK, &start],
        )?;
        return switched(&branch, &base, true);
    }
    git_stdout(ctx, &workspace, &[GIT_FETCH, ORIGIN_REMOTE, &branch])?;
    let remote_ref = format!("{ORIGIN_REMOTE}/{branch}");
    let base_ref = format!("{EXCLUDE_MARK}{ORIGIN_REMOTE}/{base}");
    let emails = git_stdout(
        ctx,
        &workspace,
        &[GIT_LOG, &remote_ref, &base_ref, GIT_AUTHOR_EMAIL_FORMAT],
    )?;
    let own_email = git_stdout(ctx, &workspace, &[GIT_CONFIG, GIT_USER_EMAIL])?;
    let foreign = foreign_authors(&emails, &own_email);
    if !foreign.is_empty() {
        return Err(typed(
            ExitStatus::NegativeVerdict,
            format!("{remote_ref} carries commits from {}", foreign.join(", ")),
        ));
    }
    git_stdout(ctx, &workspace, &[GIT_SWITCH, &branch])?;
    switched(&branch, &base, false)
}

/// Gives the main checkout back to the base branch when it is clean. Otherwise it stays as it is
/// and the issue gets a comment saying so. A dirty checkout is never an error.
pub(super) fn git_restore(ctx: &Ctx, key: &str) -> anyhow::Result<CommandOutput> {
    let workspace = ctx.workspace();
    let base = base_branch(ctx, &workspace)?;
    let current = current_branch(ctx, &workspace)?;
    if current.is_empty() || current == base {
        return output(json!({ "restored": false, "reason": "already-on-base", "base": base }));
    }
    let files = dirty_files(ctx, &workspace)?;
    if !files.is_empty() {
        let message = format!(
            "Le checkout principal reste sur `{current}` (base `{base}`) : il a des modifications locales.\n{}",
            files_text(&files)
        );
        let commented = comment_on_issue(ctx, key, &message);
        return output(json!({
            "restored": false, "reason": BLOCKED_DIRTY, "branch": current, "base": base,
            "message": message, "commented": commented, "files": files,
        }));
    }
    git_stdout(ctx, &workspace, &[GIT_SWITCH, &base])?;
    output(json!({ "restored": true, "from": current, "base": base }))
}

/// Best effort: a comment that cannot be posted never fails the restore.
fn comment_on_issue(ctx: &Ctx, key: &str, text: &str) -> bool {
    let Some(vault) = aidp_core::vault::resolve(ctx.vault_flag(), &ctx.workspace()) else {
        return false;
    };
    let Ok(store) = aidp_tickets::store::TicketStore::open(&vault.root) else {
        return false;
    };
    crate::commands::ticket_github::comment(&store, key, None, Some(text), false).is_ok()
}
