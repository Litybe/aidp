//! One git worktree per ticket at `<vault>/Pipelines/<repo>/<KEY>/.worktree`, on `feat/<KEY>`.
//! Every git call goes through [`ProcessRunner`] so tests can script it.

use std::path::{Path, PathBuf};
use std::time::Duration;

use aidp_core::process_runner::{ProcessRunner, ProcessSpec};
use aidp_core::safe_name;

use crate::models::constants::pr_fix::{PR_WORKTREE_DIR, PR_WORKTREE_DIR_PREFIX};
pub use crate::models::enums::WorkspaceKind;
pub use crate::models::Workspace;
use crate::models::WorkspaceRequest;

/// Directory name of a ticket's worktree under its ticket folder.
pub const WORKTREE_DIR: &str = ".worktree";
/// Namespace of the refs that keep work saved before a worktree is deleted.
pub const WIP_REF_PREFIX: &str = "refs/aidp-wip/";
/// Prefix of the branch every ticket works on.
pub const BRANCH_PREFIX: &str = "feat/";
/// Prefix of the branch every scratch worktree works on.
pub const SCRATCH_BRANCH_PREFIX: &str = "scratch/";
/// Prefix of a scratch worktree's folder and of its WIP ref key.
pub const SCRATCH_DIR_PREFIX: &str = "scratch-";
/// Local ignored files copied into a new worktree by default.
pub const DEFAULT_COPY: &[&str] = &[CLAUDE_LOCAL_SETTINGS, AIDP_JSON_FILES];
/// Attempts made to delete a worktree before giving up.
pub const REMOVE_ATTEMPTS: u32 = 3;
/// Pause between two failed delete attempts.
pub const REMOVE_RETRY_DELAY: Duration = Duration::from_secs(2);

const CLAUDE_LOCAL_SETTINGS: &str = ".claude/settings.local.json";
const AIDP_JSON_FILES: &str = ".aidp/*.json";
pub(crate) const PIPELINES_DIR: &str = "Pipelines";
const KEY_FIELD: &str = "key";
const GIT_PROGRAM: &str = "git";
const GIT_DIR_FLAG: &str = "-C";
const WORKTREE_LIST_FIELD: &str = "worktree ";
const HEAD_FIELD: &str = "HEAD ";
const BRANCH_FIELD: &str = "branch ";
const DETACHED_FIELD: &str = "detached";
const LOCKED_FIELD: &str = "locked";
const PRUNABLE_FIELD: &str = "prunable";
const HEADS_REF_PREFIX: &str = "refs/heads/";
const REMOTES_ORIGIN_PREFIX: &str = "refs/remotes/origin/";
const ORIGIN_REMOTE: &str = "origin";
const ORIGIN_HEAD_REF: &str = "refs/remotes/origin/HEAD";
const MASTER_BRANCH: &str = "master";
const MAIN_BRANCH: &str = "main";
const IGNORED_STATUS_PREFIX: &str = "!!";
const WIP_COMMIT_PREFIX: &str = "wip(";
const WIP_COMMIT_SUFFIX: &str = "): sauvegarde avant suppression";
const WIP_SHA_LEN: usize = 12;
const VERBATIM_PREFIX: &str = "//?/";
const WILDCARD: char = '*';
const BRANCH_ELSEWHERE_SLUG: &str = "branch-checked-out-elsewhere";
const GIT_SLUG: &str = "git";
const IO_SLUG: &str = "io";
const INVALID_KEY_SLUG: &str = "invalid-key";
const NO_DEFAULT_BRANCH_MESSAGE: &str = "no default branch found on origin";
const BACKSLASH: char = '\\';
const SLASH_STR: &str = "/";
const SLASH: char = '/';
const GIT_WORKTREE: &str = "worktree";
const GIT_LIST: &str = "list";
const GIT_STATUS: &str = "status";
const GIT_FETCH: &str = "fetch";
const GIT_ADD: &str = "add";
const GIT_COMMIT: &str = "commit";
const GIT_REV_PARSE: &str = "rev-parse";
const GIT_MERGE_BASE: &str = "merge-base";
const GIT_UPDATE_REF: &str = "update-ref";
const GIT_REMOVE: &str = "remove";
const GIT_BRANCH: &str = "branch";
const GIT_PRUNE: &str = "prune";
const GIT_MERGE: &str = "merge";
const GIT_SHOW_REF: &str = "show-ref";
const GIT_SYMBOLIC_REF: &str = "symbolic-ref";
const GIT_LS_FILES: &str = "ls-files";
const FLAG_PORCELAIN: &str = "--porcelain";
const FLAG_ALL: &str = "-A";
const FLAG_NO_VERIFY: &str = "--no-verify";
const FLAG_MESSAGE: &str = "-m";
const FLAG_FORCE: &str = "--force";
const FLAG_DELETE_FORCE: &str = "-D";
const FLAG_NEW_BRANCH: &str = "-b";
const FLAG_TRACK: &str = "--track";
const FLAG_FF_ONLY: &str = "--ff-only";
const FLAG_VERIFY: &str = "--verify";
const FLAG_QUIET: &str = "--quiet";
const FLAG_ERROR_UNMATCH: &str = "--error-unmatch";
const REV_HEAD: &str = "HEAD";
const FLAG_ABBREV_REF: &str = "--abbrev-ref";
const GIT_DIR_ENTRY: &str = ".git";

/// One entry of `git worktree list --porcelain`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeEntry {
    pub path: PathBuf,
    pub head: String,
    pub branch: Option<String>,
    pub detached: bool,
    pub locked: bool,
    pub prunable: bool,
}

/// Outcome of [`ensure`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ensured {
    pub workspace: Workspace,
    pub reused: bool,
}

/// Outcome of [`remove`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Removed {
    pub salvaged_ref: Option<String>,
}

/// Why a worktree could not be prepared.
#[derive(Debug, thiserror::Error)]
pub enum EnsureError {
    #[error("branch already checked out at {}", path.display())]
    BranchCheckedOutElsewhere { path: PathBuf },
    #[error("git failed: {0}")]
    Git(String),
    #[error("invalid ticket key: {0}")]
    InvalidKey(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl EnsureError {
    /// Stable machine-read value for the error.
    pub fn slug(&self) -> &'static str {
        match self {
            Self::BranchCheckedOutElsewhere { .. } => BRANCH_ELSEWHERE_SLUG,
            Self::Git(_) => GIT_SLUG,
            Self::InvalidKey(_) => INVALID_KEY_SLUG,
            Self::Io(_) => IO_SLUG,
        }
    }
}

/// Why a worktree could not be saved or deleted.
#[derive(Debug, thiserror::Error)]
pub enum RemoveError {
    #[error("git failed: {0}")]
    Git(String),
    #[error("invalid ticket key: {0}")]
    InvalidKey(String),
    #[error("worktree {} still in use after {attempts} attempts: {detail}", path.display())]
    Locked {
        path: PathBuf,
        attempts: u32,
        detail: String,
    },
}

struct GitOutput {
    success: bool,
    stdout: String,
    stderr: String,
}

/// Path of a ticket's worktree. The key must already satisfy [`safe_name::check`].
pub fn worktree_path(vault: &Path, repo: &str, key: &str) -> PathBuf {
    vault
        .join(PIPELINES_DIR)
        .join(repo)
        .join(key)
        .join(WORKTREE_DIR)
}

/// Ticket key owning a path shaped `.../<KEY>/.worktree`, `None` for any other path.
pub(crate) fn worktree_owner(path: &Path) -> Option<String> {
    if path.file_name()?.to_str()? != WORKTREE_DIR {
        return None;
    }
    Some(path.parent()?.file_name()?.to_str()?.to_string())
}

/// Path of a pull request's worktree: `<vault>/Pipelines/<repo>/PR-<number>/.worktree`.
pub fn pr_worktree_path(vault: &Path, repo: &str, number: u64) -> PathBuf {
    vault
        .join(PIPELINES_DIR)
        .join(repo)
        .join(format!("{PR_WORKTREE_DIR_PREFIX}{number}"))
        .join(PR_WORKTREE_DIR)
}

/// A session workspace request for an existing directory.
pub fn path_workspace_request(path: &Path) -> WorkspaceRequest {
    WorkspaceRequest {
        kind: WorkspaceKind::Path,
        ticket: None,
        name: None,
        path: Some(path.display().to_string()),
    }
}

/// The ticket worktree of `key` as a path workspace request, when it exists on disk.
pub fn ticket_path_request(
    vault: &Path,
    project_path: &Path,
    key: &str,
) -> Option<WorkspaceRequest> {
    let repo = aidp_core::vault::repo_segment(project_path);
    let path = worktree_path(vault, &repo, key);
    path.is_dir().then(|| path_workspace_request(&path))
}

/// Branch a ticket works on.
pub fn branch_for(key: &str) -> String {
    format!("{BRANCH_PREFIX}{key}")
}

/// Canonical form when the path exists, separators unified, lowercased on Windows.
pub(crate) fn normalize_path(path: &Path) -> String {
    let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let unified = resolved.to_string_lossy().replace(BACKSLASH, SLASH_STR);
    let trimmed = unified
        .strip_prefix(VERBATIM_PREFIX)
        .unwrap_or(&unified)
        .trim_end_matches(SLASH)
        .to_string();
    if cfg!(windows) {
        trimmed.to_lowercase()
    } else {
        trimmed
    }
}

/// Parse `git worktree list --porcelain`.
pub fn parse_worktree_list(porcelain: &str) -> Vec<WorktreeEntry> {
    let mut entries = Vec::new();
    let mut current: Option<WorktreeEntry> = None;
    for line in porcelain.lines() {
        if let Some(path) = line.strip_prefix(WORKTREE_LIST_FIELD) {
            entries.extend(current.take());
            current = Some(WorktreeEntry {
                path: PathBuf::from(path.trim()),
                head: String::new(),
                branch: None,
                detached: false,
                locked: false,
                prunable: false,
            });
            continue;
        }
        let Some(entry) = current.as_mut() else {
            continue;
        };
        if let Some(head) = line.strip_prefix(HEAD_FIELD) {
            entry.head = head.trim().to_string();
        } else if let Some(branch) = line.strip_prefix(BRANCH_FIELD) {
            let branch = branch.trim();
            entry.branch = Some(
                branch
                    .strip_prefix(HEADS_REF_PREFIX)
                    .unwrap_or(branch)
                    .to_string(),
            );
        } else if line.starts_with(DETACHED_FIELD) {
            entry.detached = true;
        } else if line.starts_with(LOCKED_FIELD) {
            entry.locked = true;
        } else if line.starts_with(PRUNABLE_FIELD) {
            entry.prunable = true;
        }
    }
    entries.extend(current);
    entries
}

/// Entries whose path lies under `<vault>/Pipelines/`.
pub fn under_pipelines(entries: &[WorktreeEntry], vault: &Path) -> Vec<WorktreeEntry> {
    let root = format!("{}/", normalize_path(&vault.join(PIPELINES_DIR)));
    entries
        .iter()
        .filter(|entry| normalize_path(&entry.path).starts_with(&root))
        .cloned()
        .collect()
}

/// True when `git status --porcelain` shows work worth saving: tracked changes or untracked files.
pub fn needs_salvage(porcelain_status: &str) -> bool {
    porcelain_status
        .lines()
        .any(|line| !line.trim().is_empty() && !line.starts_with(IGNORED_STATUS_PREFIX))
}

/// Default branch: the `origin/HEAD` target, then `master`, then `main`.
pub fn default_branch(
    origin_head_ref: Option<&str>,
    has_master: bool,
    has_main: bool,
) -> Option<String> {
    let from_head = origin_head_ref
        .map(|reference| reference.trim())
        .and_then(|reference| reference.strip_prefix(REMOTES_ORIGIN_PREFIX))
        .filter(|name| !name.is_empty())
        .map(str::to_string);
    from_head
        .or_else(|| has_master.then(|| MASTER_BRANCH.to_string()))
        .or_else(|| has_main.then(|| MAIN_BRANCH.to_string()))
}

fn git(runner: &dyn ProcessRunner, dir: &Path, args: &[&str]) -> Result<GitOutput, String> {
    let spec = ProcessSpec::new(GIT_PROGRAM)
        .arg(GIT_DIR_FLAG)
        .arg(dir.to_string_lossy().to_string())
        .args(args.iter().copied());
    let output = runner.output(&spec).map_err(|error| error.to_string())?;
    Ok(GitOutput {
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

fn git_checked(runner: &dyn ProcessRunner, dir: &Path, args: &[&str]) -> Result<GitOutput, String> {
    let output = git(runner, dir, args)?;
    if output.success {
        Ok(output)
    } else {
        Err(format!("{}: {}", args.join(" "), output.stderr.trim()))
    }
}

/// Attempts made on a git command that meets a lock held by another git process.
pub const LOCK_ATTEMPTS: u32 = 6;
/// Pause before the first retry on a held lock; it grows with each attempt.
pub const LOCK_RETRY_DELAY: Duration = Duration::from_millis(500);
const LOCK_FILE_MARKER: &str = ".lock";
const LOCK_HELD_MARKERS: [&str; 3] = ["File exists", "Unable to create", "another git process"];
const GIT_ERROR_MAX_CHARS: usize = 2000;

/// True when git's error says a `*.lock` file is held by another git process.
pub fn is_lock_contention(stderr: &str) -> bool {
    stderr.contains(LOCK_FILE_MARKER)
        && LOCK_HELD_MARKERS
            .iter()
            .any(|marker| stderr.contains(marker))
}

/// `git_checked`, retried a bounded number of times while git reports a held lock.
/// The error keeps git's own message (stderr), truncated.
fn git_checked_retrying(
    runner: &dyn ProcessRunner,
    sleeper: &dyn Fn(Duration),
    dir: &Path,
    args: &[&str],
) -> Result<GitOutput, String> {
    let mut attempt = 1;
    loop {
        let output = git(runner, dir, args)?;
        if output.success {
            return Ok(output);
        }
        if attempt < LOCK_ATTEMPTS && is_lock_contention(&output.stderr) {
            sleeper(LOCK_RETRY_DELAY * attempt);
            attempt += 1;
            continue;
        }
        let detail: String = output
            .stderr
            .trim()
            .chars()
            .take(GIT_ERROR_MAX_CHARS)
            .collect();
        return Err(format!("{}: {detail}", args.join(" ")));
    }
}

/// The lock that serializes the worktree creations of one project.
fn project_lock(project: &Path) -> std::sync::Arc<std::sync::Mutex<()>> {
    static LOCKS: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, std::sync::Arc<std::sync::Mutex<()>>>>,
    > = std::sync::OnceLock::new();
    let mut locks = LOCKS
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    locks.entry(normalize_path(project)).or_default().clone()
}

/// The branch checked out at `dir`, or `None` when it cannot be read or HEAD is detached.
fn current_branch(runner: &dyn ProcessRunner, dir: &Path) -> Option<String> {
    let output = git(runner, dir, &[GIT_REV_PARSE, FLAG_ABBREV_REF, REV_HEAD]).ok()?;
    let branch = output.stdout.trim();
    (output.success && !branch.is_empty() && branch != REV_HEAD).then(|| branch.to_string())
}

fn ref_exists(runner: &dyn ProcessRunner, project: &Path, reference: &str) -> bool {
    git(
        runner,
        project,
        &[GIT_SHOW_REF, FLAG_VERIFY, FLAG_QUIET, reference],
    )
    .map(|output| output.success)
    .unwrap_or(false)
}

fn resolve_base(runner: &dyn ProcessRunner, project: &Path) -> Result<String, String> {
    let origin_head = git(
        runner,
        project,
        &[GIT_SYMBOLIC_REF, FLAG_QUIET, ORIGIN_HEAD_REF],
    )
    .ok()
    .filter(|output| output.success)
    .map(|output| output.stdout);
    let has_master = ref_exists(
        runner,
        project,
        &format!("{REMOTES_ORIGIN_PREFIX}{MASTER_BRANCH}"),
    );
    let has_main = ref_exists(
        runner,
        project,
        &format!("{REMOTES_ORIGIN_PREFIX}{MAIN_BRANCH}"),
    );
    default_branch(origin_head.as_deref(), has_master, has_main)
        .map(|name| format!("{ORIGIN_REMOTE}/{name}"))
        .ok_or_else(|| NO_DEFAULT_BRANCH_MESSAGE.to_string())
}

/// Every worktree of the project, as git reports them.
fn all_entries(runner: &dyn ProcessRunner, project: &Path) -> Vec<WorktreeEntry> {
    git(runner, project, &[GIT_WORKTREE, GIT_LIST, FLAG_PORCELAIN])
        .ok()
        .filter(|output| output.success)
        .map(|output| parse_worktree_list(&output.stdout))
        .unwrap_or_default()
}

/// The project's worktrees that live under `<vault>/Pipelines/`.
pub fn list(runner: &dyn ProcessRunner, project: &Path, vault: &Path) -> Vec<WorktreeEntry> {
    under_pipelines(&all_entries(runner, project), vault)
}

/// Number of changed or untracked, non-ignored paths in a worktree.
pub fn modified_count(runner: &dyn ProcessRunner, path: &Path) -> usize {
    git(runner, path, &[GIT_STATUS, FLAG_PORCELAIN])
        .ok()
        .filter(|output| output.success)
        .map(|output| {
            output
                .stdout
                .lines()
                .filter(|line| !line.trim().is_empty() && !line.starts_with(IGNORED_STATUS_PREFIX))
                .count()
        })
        .unwrap_or(0)
}

fn worktree_workspace(path: PathBuf, branch: String) -> Workspace {
    Workspace {
        kind: WorkspaceKind::Ticket,
        path,
        branch: Some(branch),
        name: None,
    }
}

/// Reuse or create the worktree of `key` on `feat/<key>`.
/// Refuses, creating nothing, when the branch is checked out elsewhere.
/// Creations of one project are serialized: concurrent `git worktree add` on a large repository
/// contend on its index and ref locks and outlast the caller's read timeout.
pub fn ensure(
    runner: &dyn ProcessRunner,
    project: &Path,
    vault: &Path,
    repo: &str,
    key: &str,
    copy: &[String],
) -> Result<Ensured, EnsureError> {
    ensure_with_sleeper(runner, &std::thread::sleep, project, vault, repo, key, copy)
}

/// [`ensure`] for a ticket that starts from `base` (a branch of `origin`) instead of the default branch.
/// The base only matters while the ticket branch exists neither locally nor on `origin`.
pub fn ensure_based(
    runner: &dyn ProcessRunner,
    project: &Path,
    vault: &Path,
    repo: &str,
    key: &str,
    copy: &[String],
    base: Option<&str>,
) -> Result<Ensured, EnsureError> {
    ensure_with_sleeper_based(
        runner,
        &std::thread::sleep,
        project,
        vault,
        repo,
        key,
        copy,
        base,
    )
}

/// [`ensure`] with the pause between lock retries supplied by the caller.
pub fn ensure_with_sleeper(
    runner: &dyn ProcessRunner,
    sleeper: &dyn Fn(Duration),
    project: &Path,
    vault: &Path,
    repo: &str,
    key: &str,
    copy: &[String],
) -> Result<Ensured, EnsureError> {
    ensure_with_sleeper_based(runner, sleeper, project, vault, repo, key, copy, None)
}

/// [`ensure_based`] with the pause between lock retries supplied by the caller.
#[allow(clippy::too_many_arguments)]
pub fn ensure_with_sleeper_based(
    runner: &dyn ProcessRunner,
    sleeper: &dyn Fn(Duration),
    project: &Path,
    vault: &Path,
    repo: &str,
    key: &str,
    copy: &[String],
    base: Option<&str>,
) -> Result<Ensured, EnsureError> {
    safe_name::check(KEY_FIELD, key).map_err(|error| EnsureError::InvalidKey(error.to_string()))?;
    let lock = project_lock(project);
    let _guard = lock.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    ensure_locked(runner, sleeper, project, vault, repo, key, copy, base)
}

#[allow(clippy::too_many_arguments)]
fn ensure_locked(
    runner: &dyn ProcessRunner,
    sleeper: &dyn Fn(Duration),
    project: &Path,
    vault: &Path,
    repo: &str,
    key: &str,
    copy: &[String],
    base: Option<&str>,
) -> Result<Ensured, EnsureError> {
    let path = worktree_path(vault, repo, key);
    let branch = branch_for(key);
    let _ = git(runner, project, &[GIT_FETCH, ORIGIN_REMOTE]);
    let entries = all_entries(runner, project);
    let wanted = normalize_path(&path);
    if entries
        .iter()
        .any(|entry| normalize_path(&entry.path) == wanted)
    {
        return Ok(Ensured {
            workspace: worktree_workspace(path, branch),
            reused: true,
        });
    }
    // A host running the flow's external steps prepares its own checkout at the ticket path.
    if path.join(GIT_DIR_ENTRY).exists()
        && current_branch(runner, &path).as_deref() == Some(branch.as_str())
    {
        return Ok(Ensured {
            workspace: worktree_workspace(path, branch),
            reused: true,
        });
    }
    if let Some(holder) = entries
        .iter()
        .find(|entry| entry.branch.as_deref() == Some(branch.as_str()))
    {
        return Err(EnsureError::BranchCheckedOutElsewhere {
            path: holder.path.clone(),
        });
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let path_arg = path.to_string_lossy().to_string();
    if ref_exists(runner, project, &format!("{HEADS_REF_PREFIX}{branch}")) {
        git_checked_retrying(
            runner,
            sleeper,
            project,
            &[GIT_WORKTREE, GIT_ADD, &path_arg, &branch],
        )
    } else {
        // The dev flow's cleanup deletes the local branch: a later flow resumes what it pushed.
        let base = if ref_exists(runner, project, &format!("{REMOTES_ORIGIN_PREFIX}{branch}")) {
            format!("{ORIGIN_REMOTE}/{branch}")
        } else if let Some(own) = base.map(str::trim).filter(|own| !own.is_empty()) {
            let own_ref = format!("{REMOTES_ORIGIN_PREFIX}{own}");
            if !ref_exists(runner, project, &own_ref) {
                return Err(EnsureError::Git(format!(
                    "base branch `{own}` of {key} is not on origin"
                )));
            }
            format!("{ORIGIN_REMOTE}/{own}")
        } else {
            resolve_base(runner, project).map_err(EnsureError::Git)?
        };
        git_checked_retrying(
            runner,
            sleeper,
            project,
            &[
                GIT_WORKTREE,
                GIT_ADD,
                FLAG_NEW_BRANCH,
                &branch,
                &path_arg,
                &base,
            ],
        )
    }
    .map_err(EnsureError::Git)?;
    copy_local_files(runner, project, &path, copy)?;
    Ok(Ensured {
        workspace: worktree_workspace(path, branch),
        reused: false,
    })
}

fn pr_workspace(path: PathBuf, branch: &str) -> Workspace {
    Workspace {
        kind: WorkspaceKind::PrWorktree,
        path,
        branch: Some(branch.to_string()),
        name: None,
    }
}

/// Reuse or create the worktree of pull request `number` on `branch`, tracked from `origin/<branch>`.
/// Refuses, creating nothing, when the branch is checked out elsewhere.
pub fn ensure_pr_worktree(
    runner: &dyn ProcessRunner,
    project: &Path,
    vault: &Path,
    repo: &str,
    number: u64,
    branch: &str,
) -> Result<Ensured, EnsureError> {
    let path = pr_worktree_path(vault, repo, number);
    let _ = git(runner, project, &[GIT_FETCH, ORIGIN_REMOTE]);
    let entries = all_entries(runner, project);
    let wanted = normalize_path(&path);
    if entries
        .iter()
        .any(|entry| normalize_path(&entry.path) == wanted)
    {
        return Ok(Ensured {
            workspace: pr_workspace(path, branch),
            reused: true,
        });
    }
    if let Some(holder) = entries
        .iter()
        .find(|entry| entry.branch.as_deref() == Some(branch))
    {
        return Err(EnsureError::BranchCheckedOutElsewhere {
            path: holder.path.clone(),
        });
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let path_arg = path.to_string_lossy().to_string();
    let remote_branch = format!("{ORIGIN_REMOTE}/{branch}");
    if ref_exists(runner, project, &format!("{HEADS_REF_PREFIX}{branch}")) {
        git_checked(runner, project, &[GIT_WORKTREE, GIT_ADD, &path_arg, branch])
            .map_err(EnsureError::Git)?;
        let _ = git(runner, &path, &[GIT_MERGE, FLAG_FF_ONLY, &remote_branch]);
    } else {
        git_checked(
            runner,
            project,
            &[
                GIT_WORKTREE,
                GIT_ADD,
                FLAG_TRACK,
                FLAG_NEW_BRANCH,
                branch,
                &path_arg,
                &remote_branch,
            ],
        )
        .map_err(EnsureError::Git)?;
    }
    Ok(Ensured {
        workspace: pr_workspace(path, branch),
        reused: false,
    })
}

/// Save a PR worktree's unsaved work under `refs/aidp-wip/PR-<number>-<sha12>`, then delete it.
/// The local branch is kept.
pub fn remove_pr_worktree(
    runner: &dyn ProcessRunner,
    sleeper: &dyn Fn(Duration),
    project: &Path,
    path: &Path,
    number: u64,
) -> Result<Removed, RemoveError> {
    let key = format!("{PR_WORKTREE_DIR_PREFIX}{number}");
    let salvaged_ref = salvage(runner, project, path, &key)?;
    delete_worktree(runner, sleeper, project, path, None)?;
    Ok(Removed { salvaged_ref })
}

/// The branch ticket `key` starts from when it is not the default one (a bug ticket), read from its note.
pub fn ticket_base(store: &aidp_tickets::store::TicketStore, key: &str) -> Option<String> {
    let (ticket, _, _) = store.get(key).ok()?;
    ticket.github?.base_branch
}

/// [`base_sha`] for a ticket that starts from `base` (a branch of `origin`).
pub fn base_sha_from(
    runner: &dyn ProcessRunner,
    project: &Path,
    path: &Path,
    base: Option<&str>,
) -> Result<String, String> {
    let Some(own) = base.map(str::trim).filter(|own| !own.is_empty()) else {
        return base_sha(runner, project, path);
    };
    let reference = format!("{ORIGIN_REMOTE}/{own}");
    let output = git_checked(runner, path, &[GIT_MERGE_BASE, REV_HEAD, &reference])
        .or_else(|_| git_checked(runner, path, &[GIT_REV_PARSE, REV_HEAD]))?;
    Ok(output.stdout.trim().to_string())
}

/// Commit the worktree branched from: the merge base with the default branch, else its HEAD.
pub fn base_sha(runner: &dyn ProcessRunner, project: &Path, path: &Path) -> Result<String, String> {
    let merge_base = resolve_base(runner, project)
        .ok()
        .and_then(|base| git_checked(runner, path, &[GIT_MERGE_BASE, REV_HEAD, &base]).ok());
    let output = match merge_base {
        Some(output) => output,
        None => git_checked(runner, path, &[GIT_REV_PARSE, REV_HEAD])?,
    };
    Ok(output.stdout.trim().to_string())
}

/// The worktree of `branch` when it is `feat/<KEY>` and that worktree exists.
pub fn existing_for_branch(
    runner: &dyn ProcessRunner,
    project: &Path,
    vault: &Path,
    repo: &str,
    branch: &str,
) -> Option<Workspace> {
    let key = branch.strip_prefix(BRANCH_PREFIX)?;
    safe_name::check(KEY_FIELD, key).ok()?;
    let wanted = normalize_path(&worktree_path(vault, repo, key));
    list(runner, project, vault)
        .into_iter()
        .find(|entry| normalize_path(&entry.path) == wanted)
        .map(|entry| worktree_workspace(entry.path, branch.to_string()))
}

/// Path of a scratch worktree: `<vault>/Pipelines/<repo>/scratch-<name>/.worktree`.
pub fn scratch_path(vault: &Path, repo: &str, name: &str) -> PathBuf {
    vault
        .join(PIPELINES_DIR)
        .join(repo)
        .join(format!("{SCRATCH_DIR_PREFIX}{name}"))
        .join(WORKTREE_DIR)
}

/// Branch a scratch worktree works on.
pub fn scratch_branch_for(name: &str) -> String {
    format!("{SCRATCH_BRANCH_PREFIX}{name}")
}

fn scratch_workspace(path: PathBuf, name: &str) -> Workspace {
    Workspace {
        kind: WorkspaceKind::Scratch,
        path,
        branch: Some(scratch_branch_for(name)),
        name: Some(name.to_string()),
    }
}

/// Reuse or create the scratch worktree of `name` on `scratch/<name>`, branched from `origin/<default>`.
/// Fetches only when the worktree has to be created.
pub fn ensure_scratch(
    runner: &dyn ProcessRunner,
    project: &Path,
    vault: &Path,
    repo: &str,
    name: &str,
) -> Result<Ensured, EnsureError> {
    let path = scratch_path(vault, repo, name);
    let branch = scratch_branch_for(name);
    let wanted = normalize_path(&path);
    let entries = all_entries(runner, project);
    if entries
        .iter()
        .any(|entry| normalize_path(&entry.path) == wanted)
    {
        return Ok(Ensured {
            workspace: scratch_workspace(path, name),
            reused: true,
        });
    }
    if let Some(holder) = entries
        .iter()
        .find(|entry| entry.branch.as_deref() == Some(branch.as_str()))
    {
        return Err(EnsureError::BranchCheckedOutElsewhere {
            path: holder.path.clone(),
        });
    }
    git_checked(runner, project, &[GIT_FETCH, ORIGIN_REMOTE]).map_err(EnsureError::Git)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let base = resolve_base(runner, project).map_err(EnsureError::Git)?;
    let path_arg = path.to_string_lossy().to_string();
    git_checked(
        runner,
        project,
        &[
            GIT_WORKTREE,
            GIT_ADD,
            FLAG_NEW_BRANCH,
            &branch,
            &path_arg,
            &base,
        ],
    )
    .map_err(EnsureError::Git)?;
    Ok(Ensured {
        workspace: scratch_workspace(path, name),
        reused: false,
    })
}

/// Save a scratch worktree's unsaved work under `refs/aidp-wip/scratch-<name>-<sha12>`, then delete it.
/// A clean tree is deleted directly.
pub fn release_scratch(
    runner: &dyn ProcessRunner,
    sleeper: &dyn Fn(Duration),
    project: &Path,
    workspace: &Workspace,
) -> Result<Removed, RemoveError> {
    let name = workspace.name.as_deref().unwrap_or_default();
    let key = format!("{SCRATCH_DIR_PREFIX}{name}");
    let salvaged_ref = salvage(runner, project, &workspace.path, &key)?;
    delete_worktree(
        runner,
        sleeper,
        project,
        &workspace.path,
        Some(&scratch_branch_for(name)),
    )?;
    Ok(Removed { salvaged_ref })
}

fn wildcard_matches(pattern: &str, text: &str) -> bool {
    match pattern.split_once(WILDCARD) {
        None => pattern == text,
        Some((head, tail)) => {
            let Some(rest) = text.strip_prefix(head) else {
                return false;
            };
            (0..=rest.len())
                .filter(|index| rest.is_char_boundary(*index))
                .any(|index| wildcard_matches(tail, &rest[index..]))
        }
    }
}

/// Project-relative files a copy pattern designates. A wildcard is honoured in the last segment only.
fn expand_pattern(project: &Path, pattern: &str) -> Vec<String> {
    let (dir_part, file_part) = pattern.rsplit_once(SLASH).unwrap_or(("", pattern));
    if !file_part.contains(WILDCARD) {
        return vec![pattern.to_string()];
    }
    let Ok(read) = std::fs::read_dir(project.join(dir_part)) else {
        return Vec::new();
    };
    let mut found: Vec<String> = read
        .flatten()
        .filter_map(|item| item.file_name().into_string().ok())
        .filter(|name| wildcard_matches(file_part, name))
        .map(|name| {
            if dir_part.is_empty() {
                name
            } else {
                format!("{dir_part}{SLASH_STR}{name}")
            }
        })
        .collect();
    found.sort();
    found
}

fn is_tracked(runner: &dyn ProcessRunner, project: &Path, relative: &str) -> bool {
    git(
        runner,
        project,
        &[GIT_LS_FILES, FLAG_ERROR_UNMATCH, relative],
    )
    .map(|output| output.success)
    .unwrap_or(true)
}

/// Copy each existing, untracked file matching `patterns` into the worktree, never overwriting.
fn copy_local_files(
    runner: &dyn ProcessRunner,
    project: &Path,
    worktree: &Path,
    patterns: &[String],
) -> Result<(), EnsureError> {
    for relative in patterns
        .iter()
        .flat_map(|pattern| expand_pattern(project, pattern))
    {
        let source = project.join(&relative);
        let target = worktree.join(&relative);
        if !source.is_file() || target.exists() || is_tracked(runner, project, &relative) {
            continue;
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(&source, &target)?;
    }
    Ok(())
}

/// Commit unsaved work and pin it under `refs/aidp-wip/<KEY>-<sha12>`, never pushing.
/// Returns `None` when nothing was worth saving.
pub fn salvage(
    runner: &dyn ProcessRunner,
    project: &Path,
    path: &Path,
    key: &str,
) -> Result<Option<String>, RemoveError> {
    safe_name::check(KEY_FIELD, key).map_err(|error| RemoveError::InvalidKey(error.to_string()))?;
    let status =
        git_checked(runner, path, &[GIT_STATUS, FLAG_PORCELAIN]).map_err(RemoveError::Git)?;
    if !needs_salvage(&status.stdout) {
        return Ok(None);
    }
    let message = format!("{WIP_COMMIT_PREFIX}{key}{WIP_COMMIT_SUFFIX}");
    git_checked(runner, path, &[GIT_ADD, FLAG_ALL]).map_err(RemoveError::Git)?;
    git_checked(
        runner,
        path,
        &[GIT_COMMIT, FLAG_NO_VERIFY, FLAG_MESSAGE, &message],
    )
    .map_err(RemoveError::Git)?;
    let head = git_checked(runner, path, &[GIT_REV_PARSE, REV_HEAD]).map_err(RemoveError::Git)?;
    let sha = head.stdout.trim().to_string();
    let short: String = sha.chars().take(WIP_SHA_LEN).collect();
    let reference = format!("{WIP_REF_PREFIX}{key}-{short}");
    git_checked(runner, project, &[GIT_UPDATE_REF, &reference, &sha]).map_err(RemoveError::Git)?;
    Ok(Some(reference))
}

/// Delete the worktree at `path` with retries, then its `branch` and any stale registration.
fn delete_worktree(
    runner: &dyn ProcessRunner,
    sleeper: &dyn Fn(Duration),
    project: &Path,
    path: &Path,
    branch: Option<&str>,
) -> Result<(), RemoveError> {
    let path_arg = path.to_string_lossy().to_string();
    let mut last_detail = String::new();
    let mut deleted = false;
    for attempt in 1..=REMOVE_ATTEMPTS {
        match git_checked(
            runner,
            project,
            &[GIT_WORKTREE, GIT_REMOVE, FLAG_FORCE, &path_arg],
        ) {
            Ok(_) => {
                deleted = true;
                break;
            }
            Err(detail) => last_detail = detail,
        }
        if attempt < REMOVE_ATTEMPTS {
            sleeper(REMOVE_RETRY_DELAY);
        }
    }
    if !deleted {
        return Err(RemoveError::Locked {
            path: path.to_path_buf(),
            attempts: REMOVE_ATTEMPTS,
            detail: last_detail,
        });
    }
    if let Some(branch) = branch {
        let _ = git(runner, project, &[GIT_BRANCH, FLAG_DELETE_FORCE, branch]);
    }
    git_checked(runner, project, &[GIT_WORKTREE, GIT_PRUNE]).map_err(RemoveError::Git)?;
    Ok(())
}

/// Save unsaved work, then delete the worktree and its branch.
/// Deleting is attempted [`REMOVE_ATTEMPTS`] times, calling `sleeper` between attempts.
pub fn remove(
    runner: &dyn ProcessRunner,
    sleeper: &dyn Fn(Duration),
    project: &Path,
    path: &Path,
    key: &str,
) -> Result<Removed, RemoveError> {
    let salvaged_ref = salvage(runner, project, path, key)?;
    delete_worktree(runner, sleeper, project, path, Some(&branch_for(key)))?;
    Ok(Removed { salvaged_ref })
}

#[cfg(test)]
#[path = "../tests/unit/worktrees_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../tests/unit/worktrees_git_tests.rs"]
mod worktrees_git_tests;
