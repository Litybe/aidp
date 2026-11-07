//! Runs a plan task in its own detached git worktree, then applies its changes back.
//! Every git command takes the ticket worktree or the isolated tree as its cwd, so it works
//! from a checkout the host prepared too.

use super::scope_guard::is_exempt;
use crate::models::constants::task_isolation::{
    ADD_ARG, ALLOW_EMPTY_FLAG, ALL_FLAG, APPLY_ARG, AUTHOR_EMAIL_CONFIG, AUTHOR_NAME_CONFIG,
    BASE_COMMIT_MESSAGE, BINARY_FLAG, CACHED_FLAG, CHECK_FLAG, COMMIT_ARG, CONFIG_FLAG,
    DETACH_FLAG, DIFF_ARG, ERROR_PREFIX, EXCLUDE_STANDARD_FLAG, FORCE_FLAG, FULL_INDEX_FLAG,
    GIT_PROGRAM, ISOLATION_DIR, LS_FILES_ARG, MESSAGE_FLAG, NO_AUTOCRLF_CONFIG, NO_SIGN_CONFIG,
    NO_VERIFY_FLAG, OTHERS_FLAG, PATCH_EXTENSION, PATCH_FAILED_PREFIX, PATH_ERROR_SUFFIXES,
    PATH_LINE_SEPARATOR, PRUNE_ARG, QUIET_FLAG, REMOVE_ARG, SHOW_TOPLEVEL_FLAG, STATE_PATCH_SUFFIX,
    WORKTREE_ARG,
};
use crate::models::constants::task_scope::{
    HEAD_REF, NUL_SEPARATED_FLAG, NUL_SEPARATOR, REV_PARSE_ARG,
};
use crate::models::enums::apply_verdict::ApplyVerdict;
use crate::models::task_worktree::TaskWorktree;
use aidp_core::process_runner::{ProcessRunner, ProcessSpec};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tracing::warn;

/// Where the isolated trees of a run live.
pub fn isolation_root(run_dir: &Path) -> PathBuf {
    run_dir.join(ISOLATION_DIR)
}

/// True when the tasks of a stage run at the same time and must not share a worktree.
pub fn should_isolate(alone: bool, effective_parallel: usize) -> bool {
    !alone && effective_parallel > 1
}

/// The root of the checkout `ticket` belongs to: a patch names paths from that root, so `git apply`
/// must run there. Falls back to `ticket` when git names no folder.
fn checkout_root(ticket: &Path, runner: &dyn ProcessRunner) -> PathBuf {
    output_of(
        runner,
        &git(ticket).args([REV_PARSE_ARG, SHOW_TOPLEVEL_FLAG]),
    )
    .ok()
    .map(|stdout| PathBuf::from(String::from_utf8_lossy(&stdout).trim()))
    .filter(|root| root.is_dir())
    .unwrap_or_else(|| ticket.to_path_buf())
}

/// Builds the isolated worktree of `task_id` from the current state of `ticket`: its HEAD plus
/// its uncommitted changes, committed as a base so the tree starts clean.
/// `exempt_prefix` (the run directory when it lives in the ticket tree) is not copied.
/// A leftover tree of the same task is removed first; a failure removes what was built.
pub fn prepare(
    ticket: &Path,
    root: &Path,
    task_id: &str,
    exempt_prefix: Option<&str>,
    runner: &dyn ProcessRunner,
) -> anyhow::Result<TaskWorktree> {
    let ticket = checkout_root(ticket, runner);
    let ticket = ticket.as_path();
    let path = root.join(task_id);
    release(ticket, &path, runner);
    let built = build(ticket, root, task_id, &path, exempt_prefix, runner);
    if built.is_err() {
        release(ticket, &path, runner);
    }
    built
}

fn build(
    ticket: &Path,
    root: &Path,
    task_id: &str,
    path: &Path,
    exempt_prefix: Option<&str>,
    runner: &dyn ProcessRunner,
) -> anyhow::Result<TaskWorktree> {
    std::fs::create_dir_all(root)?;
    let target = path.to_string_lossy().into_owned();
    run_checked(
        runner,
        &git(ticket).args([
            WORKTREE_ARG,
            ADD_ARG,
            DETACH_FLAG,
            target.as_str(),
            HEAD_REF,
        ]),
    )?;
    let state = output_of(
        runner,
        &git(ticket).args([
            CONFIG_FLAG,
            NO_AUTOCRLF_CONFIG,
            DIFF_ARG,
            HEAD_REF,
            BINARY_FLAG,
            FULL_INDEX_FLAG,
        ]),
    )?;
    let listing = output_of(
        runner,
        &git(ticket).args([
            LS_FILES_ARG,
            OTHERS_FLAG,
            EXCLUDE_STANDARD_FLAG,
            NUL_SEPARATED_FLAG,
        ]),
    )?;
    let untracked: Vec<String> = String::from_utf8_lossy(&listing)
        .split(NUL_SEPARATOR)
        .filter(|entry| !entry.is_empty())
        .filter(|entry| !is_exempt(entry, exempt_prefix))
        .map(str::to_string)
        .collect();
    carry_state(runner, path, root, task_id, &state)?;
    for entry in &untracked {
        let destination = path.join(entry);
        if let Some(parent) = destination.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::copy(ticket.join(entry), destination)?;
    }
    if !state.is_empty() || !untracked.is_empty() {
        commit_base(runner, path)?;
    }
    let head = output_of(runner, &git(path).args([REV_PARSE_ARG, HEAD_REF]))?;
    Ok(TaskWorktree {
        path: path.to_path_buf(),
        base: String::from_utf8_lossy(&head).trim().to_string(),
    })
}

/// Applies the uncommitted tracked changes of the ticket tree inside the isolated tree.
fn carry_state(
    runner: &dyn ProcessRunner,
    path: &Path,
    root: &Path,
    task_id: &str,
    state: &[u8],
) -> anyhow::Result<()> {
    if state.is_empty() {
        return Ok(());
    }
    let patch = root.join(format!("{task_id}{STATE_PATCH_SUFFIX}"));
    std::fs::write(&patch, state)?;
    let applied = run_checked(
        runner,
        &git(path).args([
            CONFIG_FLAG,
            NO_AUTOCRLF_CONFIG,
            APPLY_ARG,
            patch.to_string_lossy().as_ref(),
        ]),
    );
    let _ = std::fs::remove_file(&patch);
    applied
}

/// Commits the carried state, so the isolated tree starts clean and its HEAD is the base.
fn commit_base(runner: &dyn ProcessRunner, path: &Path) -> anyhow::Result<()> {
    run_checked(
        runner,
        &git(path).args([CONFIG_FLAG, NO_AUTOCRLF_CONFIG, ADD_ARG, ALL_FLAG]),
    )?;
    run_checked(
        runner,
        &git(path).args([
            CONFIG_FLAG,
            AUTHOR_NAME_CONFIG,
            CONFIG_FLAG,
            AUTHOR_EMAIL_CONFIG,
            CONFIG_FLAG,
            NO_SIGN_CONFIG,
            COMMIT_ARG,
            NO_VERIFY_FLAG,
            ALLOW_EMPTY_FLAG,
            QUIET_FLAG,
            MESSAGE_FLAG,
            BASE_COMMIT_MESSAGE,
        ]),
    )
}

/// Applies what the task changed in its isolated tree onto `ticket`. The ticket tree is checked
/// first, so a conflict leaves it untouched. `written` names the paths when git names none.
pub fn apply_back(
    ticket: &Path,
    tree: &TaskWorktree,
    written: &[String],
    runner: &dyn ProcessRunner,
) -> anyhow::Result<ApplyVerdict> {
    let ticket = checkout_root(ticket, runner);
    let ticket = ticket.as_path();
    run_checked(
        runner,
        &git(&tree.path).args([CONFIG_FLAG, NO_AUTOCRLF_CONFIG, ADD_ARG, ALL_FLAG]),
    )?;
    let patch = output_of(
        runner,
        &git(&tree.path).args([
            CONFIG_FLAG,
            NO_AUTOCRLF_CONFIG,
            DIFF_ARG,
            CACHED_FLAG,
            BINARY_FLAG,
            FULL_INDEX_FLAG,
            tree.base.as_str(),
        ]),
    )?;
    if patch.is_empty() {
        return Ok(ApplyVerdict::Applied);
    }
    let file = patch_file(&tree.path);
    std::fs::write(&file, &patch)?;
    let target = file.to_string_lossy().into_owned();
    for check in [true, false] {
        let mut spec = git(ticket).args([CONFIG_FLAG, NO_AUTOCRLF_CONFIG, APPLY_ARG]);
        if check {
            spec = spec.arg(CHECK_FLAG);
        }
        let output = runner.output(&spec.arg(target.as_str()))?;
        if !output.status.success() {
            let named = conflict_paths(&String::from_utf8_lossy(&output.stderr));
            return Ok(ApplyVerdict::Conflict(if named.is_empty() {
                written.to_vec()
            } else {
                named
            }));
        }
    }
    Ok(ApplyVerdict::Applied)
}

/// Paths git names in the stderr of a failed `git apply`.
pub fn conflict_paths(stderr: &str) -> Vec<String> {
    let mut paths: Vec<String> = Vec::new();
    for line in stderr.lines() {
        let Some(text) = line.trim().strip_prefix(ERROR_PREFIX) else {
            continue;
        };
        let path = if let Some(rest) = text.strip_prefix(PATCH_FAILED_PREFIX) {
            rest.rsplit_once(PATH_LINE_SEPARATOR)
                .map_or(rest, |(path, _)| path)
        } else if let Some(suffix) = PATH_ERROR_SUFFIXES
            .iter()
            .find(|suffix| text.ends_with(**suffix))
        {
            &text[..text.len() - suffix.len()]
        } else {
            continue;
        };
        if !paths.iter().any(|known| known == path) {
            paths.push(path.to_string());
        }
    }
    paths
}

/// Removes the isolated tree and its patch file. Failures are logged, never returned.
pub fn release(ticket: &Path, path: &Path, runner: &dyn ProcessRunner) {
    remove_tree(ticket, path, runner);
    prune(ticket, runner);
    let _ = std::fs::remove_file(patch_file(path));
}

/// Removes every leftover tree under `root`, left by a run that died. It deletes every entry, so
/// it must only run at the start of a stage: stages run one at a time, so no task of the same run
/// is using a tree then.
pub fn sweep(ticket: &Path, root: &Path, runner: &dyn ProcessRunner) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    let mut removed = false;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            remove_tree(ticket, &path, runner);
        } else {
            let _ = std::fs::remove_file(&path);
        }
        removed = true;
    }
    if removed {
        prune(ticket, runner);
    }
}

fn remove_tree(ticket: &Path, path: &Path, runner: &dyn ProcessRunner) {
    let target = path.to_string_lossy().into_owned();
    let spec = git(ticket).args([WORKTREE_ARG, REMOVE_ARG, FORCE_FLAG, target.as_str()]);
    if let Err(error) = run_checked(runner, &spec) {
        warn!(path = %path.display(), %error, "could not remove the isolated task worktree");
    }
    if path.exists() {
        if let Err(error) = std::fs::remove_dir_all(path) {
            warn!(path = %path.display(), %error, "could not delete the isolated task folder");
        }
    }
}

fn prune(ticket: &Path, runner: &dyn ProcessRunner) {
    if let Err(error) = run_checked(runner, &git(ticket).args([WORKTREE_ARG, PRUNE_ARG])) {
        warn!(%error, "could not prune the worktree list");
    }
}

fn patch_file(tree: &Path) -> PathBuf {
    let mut name = tree.as_os_str().to_os_string();
    name.push(PATCH_EXTENSION);
    PathBuf::from(name)
}

fn git(cwd: &Path) -> ProcessSpec {
    ProcessSpec::new(GIT_PROGRAM).cwd(cwd)
}

fn output_of(runner: &dyn ProcessRunner, spec: &ProcessSpec) -> anyhow::Result<Vec<u8>> {
    let output = runner.output(spec)?;
    if output.status.success() {
        return Ok(output.stdout);
    }
    anyhow::bail!(
        "{} {} failed: {}",
        spec.program,
        spec.args.join(" "),
        String::from_utf8_lossy(&output.stderr).trim()
    )
}

fn run_checked(runner: &dyn ProcessRunner, spec: &ProcessSpec) -> anyhow::Result<()> {
    output_of(runner, spec).map(|_| ())
}

/// Releases the isolated tree when dropped: on success, failure, cancel and panic alike.
/// The release runs under `lock`, the mutex that also guards `prepare` and `apply_back`, so git
/// never removes a worktree while another task adds one.
pub struct IsolatedTask<'a> {
    pub tree: TaskWorktree,
    ticket: &'a Path,
    runner: &'a dyn ProcessRunner,
    lock: &'a Mutex<()>,
}

impl<'a> IsolatedTask<'a> {
    pub fn new(
        ticket: &'a Path,
        tree: TaskWorktree,
        runner: &'a dyn ProcessRunner,
        lock: &'a Mutex<()>,
    ) -> Self {
        Self {
            tree,
            ticket,
            runner,
            lock,
        }
    }
}

impl Drop for IsolatedTask<'_> {
    fn drop(&mut self) {
        let _guard = self
            .lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        release(self.ticket, &self.tree.path, self.runner);
    }
}

#[cfg(test)]
#[path = "../../tests/unit/task_isolation_tests.rs"]
mod tests;
