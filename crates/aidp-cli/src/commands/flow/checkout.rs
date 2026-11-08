//! `flow git-checkout` and `flow git-restore`: a flow with `workspace: project` works in the main
//! checkout of the project. The first puts it on the branch of the ticket (created from the base
//! branch when absent), the second gives it back to the base branch.

use super::{
    branch_of, flow_ticket, foreign_authors, git_output, git_stdout, git_succeeds, issue_type_of,
    typed, verify_args, CommandOutput, Ctx, GIT_FAILED_MESSAGE,
};
use crate::models::constants::flow_git::*;
use aidp_core::ExitStatus;
use aidp_gateway::native_runs::scope_guard::{is_unity_editor_owned, is_unity_project};
use serde_json::{json, Value};
use std::path::Path;

const GIT_STATUS: &str = "status";
const GIT_PORCELAIN: &str = "--porcelain";
const GIT_NUL_SEPARATED: &str = "-z";
const GIT_UNTRACKED_ALL: &str = "-uall";
const STATUS_CODE_LEN: usize = 2;
const UNTRACKED_CODE: &str = "??";
const META_SUFFIX: &str = ".meta";
const OVERWRITTEN_HEADER: &str = "untracked working tree files would be overwritten";
const GIT_SHOW_CURRENT: [&str; 2] = ["branch", "--show-current"];
const GIT_ORIGIN_HEAD: [&str; 3] = ["symbolic-ref", "--short", "refs/remotes/origin/HEAD"];
const GIT_NO_TRACK: &str = "--no-track";
const BASE_CANDIDATES: [&str; 2] = ["main", "master"];
const GIT_FORCE_FLAG: &str = "--force";
const GIT_UPDATE_INDEX: &str = "update-index";
const SKIP_WORKTREE: &str = "--skip-worktree";
const NO_SKIP_WORKTREE: &str = "--no-skip-worktree";
const EDITOR_VERSION_FILE: &str = "ProjectSettings/ProjectVersion.txt";
const GIT_PULL: &str = "pull";
const GIT_FF_ONLY: &str = "--ff-only";
const GIT_MERGE: &str = "merge";
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

/// The paths `git status` lists (untracked included), minus the Unity editor's own noise.
fn dirty_files(ctx: &Ctx, workspace: &Path) -> anyhow::Result<Vec<String>> {
    let raw = git_stdout(
        ctx,
        workspace,
        &[
            GIT_STATUS,
            GIT_PORCELAIN,
            GIT_NUL_SEPARATED,
            GIT_UNTRACKED_ALL,
        ],
    )?;
    let unity = is_unity_project(workspace);
    Ok(parse_status(&raw)
        .into_iter()
        .filter(|(code, path)| !(unity && is_editor_noise(code, path)))
        .map(|(code, path)| format!("{code} {path}"))
        .collect())
}

/// The `(XY, path)` entries of `git status --porcelain -z`. A rename or copy entry is followed by
/// its origin path, which is dropped.
fn parse_status(raw: &str) -> Vec<(String, String)> {
    let mut entries = Vec::new();
    let mut fields = raw.split('\0');
    while let Some(field) = fields.next() {
        let Some((code, path)) = field
            .split_at_checked(STATUS_CODE_LEN)
            .and_then(|(code, rest)| rest.strip_prefix(' ').map(|path| (code, path)))
        else {
            continue;
        };
        if code.contains(['R', 'C']) {
            fields.next();
        }
        entries.push((code.to_string(), path.to_string()));
    }
    entries
}

/// What an open Unity editor produces on its own: the `.meta` of an asset that arrived without
/// one (untracked) and the files of `UNITY_EDITOR_OWNED`.
fn is_editor_noise(code: &str, path: &str) -> bool {
    (code == UNTRACKED_CODE && path.ends_with(META_SUFFIX)) || is_unity_editor_owned(path)
}

/// The untracked `.meta` files a failed `git switch` says it would overwrite.
fn overwritten_metas(stderr: &str) -> Vec<String> {
    let mut metas = Vec::new();
    let mut listing = false;
    for line in stderr.lines() {
        if line.contains(OVERWRITTEN_HEADER) {
            listing = true;
        } else if listing && line.starts_with('\t') {
            let path = line.trim();
            if path.ends_with(META_SUFFIX) && !path.contains("..") {
                metas.push(path.to_string());
            }
        } else if listing {
            listing = false;
        }
    }
    metas
}

/// `git switch`. In a Unity project, when untracked `.meta` files of the editor block it because
/// the target branch tracks them, those files are removed and the switch is tried once more.
#[cfg(test)]
fn switch(ctx: &Ctx, workspace: &Path, args: &[&str]) -> anyhow::Result<()> {
    switch_with(ctx, workspace, args, false)
}

/// `switch`, with `--force` (local changes of tracked files are discarded) when `force` is set.
fn switch_with(ctx: &Ctx, workspace: &Path, args: &[&str], force: bool) -> anyhow::Result<()> {
    let mut forced: Vec<&str> = args.to_vec();
    if force {
        forced.insert(1, GIT_FORCE_FLAG);
    }
    let args = forced.as_slice();
    let output = git_output(ctx, workspace, args)?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let metas = if is_unity_project(workspace) {
        overwritten_metas(&stderr)
    } else {
        Vec::new()
    };
    if metas.is_empty() {
        return Err(typed(
            ExitStatus::UpstreamFailure,
            format!("{GIT_FAILED_MESSAGE}: {}", stderr.trim()),
        ));
    }
    for meta in metas {
        let _ = std::fs::remove_file(workspace.join(meta));
    }
    git_stdout(ctx, workspace, args).map(|_| ())
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
    let force = is_test_checkout(&ctx.workspace());
    keeping_editor_version(ctx, force, || checkout_in(ctx, key, force))
}

/// True when `workspace` is the test checkout of a registered project: a disposable clone owned by
/// the Unity harness, whose local changes (the editor rewrites tracked files) are discarded.
fn is_test_checkout(workspace: &Path) -> bool {
    let Ok(state_dir) = aidp_gateway::paths::state_dir() else {
        return false;
    };
    let wanted = normalized(workspace);
    aidp_gateway::projects::Projects::load(&state_dir)
        .list()
        .iter()
        .filter_map(|project| project.test_checkout.as_deref())
        .any(|path| normalized(Path::new(path)) == wanted)
}

fn normalized(path: &Path) -> String {
    path.to_string_lossy()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_lowercase()
}

/// Runs `body`; with `force`, the editor's `ProjectSettings/ProjectVersion.txt` survives the
/// switch (saved before, written back after) and is marked `skip-worktree` so it stays clean and
/// the editor never asks for a migration.
fn keeping_editor_version<T>(
    ctx: &Ctx,
    force: bool,
    body: impl FnOnce() -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    if !force {
        return body();
    }
    let workspace = ctx.workspace();
    let file = workspace.join(EDITOR_VERSION_FILE);
    let saved = std::fs::read(&file).ok();
    let _ = git_output(
        ctx,
        &workspace,
        &[GIT_UPDATE_INDEX, NO_SKIP_WORKTREE, EDITOR_VERSION_FILE],
    );
    let result = body();
    if let Some(bytes) = saved {
        let _ = std::fs::write(&file, bytes);
        let _ = git_output(
            ctx,
            &workspace,
            &[GIT_UPDATE_INDEX, SKIP_WORKTREE, EDITOR_VERSION_FILE],
        );
    }
    result
}

fn checkout_in(ctx: &Ctx, key: &str, force: bool) -> anyhow::Result<CommandOutput> {
    let workspace = ctx.workspace();
    // The branch comes from a dev worktree and was pushed: it may not exist in this clone yet.
    // Best effort, a clone without network goes on with the refs it has.
    let _ = git_output(ctx, &workspace, &[GIT_FETCH, ORIGIN_REMOTE]);
    let branch = ticket_branch(ctx, key)?;
    let base = base_branch(ctx, &workspace)?;
    let current = current_branch(ctx, &workspace)?;
    if current == branch {
        return switched(&branch, &base, false);
    }
    let files = if force {
        Vec::new()
    } else {
        dirty_files(ctx, &workspace)?
    };
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
        switch_with(ctx, &workspace, &[GIT_SWITCH, &branch], force)?;
        // Pushed fixes of the dev worktree reach a branch reused from an earlier test.
        let remote_ref = format!("{ORIGIN_REMOTE}/{branch}");
        let _ = git_output(ctx, &workspace, &[GIT_MERGE, GIT_FF_ONLY, &remote_ref]);
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
        switch_with(
            ctx,
            &workspace,
            &[GIT_SWITCH, GIT_CREATE_FLAG, &branch, GIT_NO_TRACK, &start],
            force,
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
    switch_with(ctx, &workspace, &[GIT_SWITCH, &branch], force)?;
    switched(&branch, &base, false)
}

/// Gives the main checkout back to the base branch when it is clean. Otherwise it stays as it is
/// and the issue gets a comment saying so. A dirty checkout is never an error.
pub(super) fn git_restore(ctx: &Ctx, key: &str) -> anyhow::Result<CommandOutput> {
    let force = is_test_checkout(&ctx.workspace());
    keeping_editor_version(ctx, force, || restore_in(ctx, key, force))
}

fn restore_in(ctx: &Ctx, key: &str, force: bool) -> anyhow::Result<CommandOutput> {
    let workspace = ctx.workspace();
    let base = base_branch(ctx, &workspace)?;
    let current = current_branch(ctx, &workspace)?;
    if current.is_empty() || current == base {
        let pulled = pull_base(ctx, &workspace);
        return output(
            json!({ "restored": false, "reason": "already-on-base", "base": base, "pulled": pulled }),
        );
    }
    let files = if force {
        Vec::new()
    } else {
        dirty_files(ctx, &workspace)?
    };
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
    switch_with(ctx, &workspace, &[GIT_SWITCH, &base], force)?;
    let pulled = pull_base(ctx, &workspace);
    output(json!({ "restored": true, "from": current, "base": base, "pulled": pulled }))
}

/// `git pull --ff-only` on the base branch, so the next test starts from `origin/<base>`.
/// Best effort: false when it could not (no network, diverged, dirty).
fn pull_base(ctx: &Ctx, workspace: &Path) -> bool {
    git_succeeds(ctx, workspace, &[GIT_PULL, GIT_FF_ONLY]).unwrap_or(false)
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

#[cfg(test)]
#[path = "../../../tests/unit/flow_checkout_tests.rs"]
mod tests;
