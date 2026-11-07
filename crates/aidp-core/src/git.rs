//! The one substitutable point of contact with the `git` binary: [`GitCli`]
//! spawns it, [`RecordingGit`] fakes it for tests.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::Ordering;

use crate::gitlog::Commit;
use crate::models::constants::git::{
    ABBREV_REF_FLAG, COMMITTER_DATE_FORMAT_FLAG, GIT_DIR_FLAG, GIT_PROGRAM, HEAD_REF,
    IS_INSIDE_WORK_TREE_FLAG, LAST_ONE_FLAG, LIST, LOG, LOG_FORMAT_FLAG, NAME_ONLY_FLAG,
    PATH_SEPARATOR_ARG, PORCELAIN_FLAG, RECORDING_GIT_POISONED, REV_PARSE, WORKTREE,
    WORKTREE_LINE_PREFIX,
};
use crate::models::port::SharedPort;

pub use crate::models::git::{GitCallCounts, RecordingGit};

/// Every git read this crate's production code needs, behind one port.
pub trait Git: SharedPort {
    /// `git rev-parse HEAD`. `None` on any failure.
    fn head(&self, workspace: &Path) -> Option<String>;

    /// `git rev-parse --abbrev-ref HEAD`. `None` on any failure.
    fn branch(&self, workspace: &Path) -> Option<String>;

    /// `git log -n<n> [--since=<since>] --format=%H%x1f%s%x1f%aI --name-only`.
    fn log(&self, workspace: &Path, n: usize, since: Option<&str>) -> anyhow::Result<Vec<Commit>>;

    /// Every path `workspace` has changed relative to `base`, combining
    /// committed diff and working-tree status.
    fn changed(&self, workspace: &Path, base: &str) -> Vec<String>;

    /// The file's most recent commit timestamp (RFC3339, committer date), or
    /// `None` for anything short of a clean success with non-empty output.
    fn last_commit_date(&self, workspace: &Path, file: &str) -> Option<String>;

    /// Whether `workspace` is usable as a git working tree at all.
    fn is_usable(&self, workspace: &Path) -> bool;

    /// Linked worktrees registered for the repository at `workspace`,
    /// excluding `workspace` itself. Empty on any failure.
    fn worktree_list(&self, workspace: &Path) -> Vec<PathBuf>;
}

/// Run `git <args>` in `workspace`, returning stdout on success, `None` on
/// any failure or empty output. Trims only the trailing newline: a porcelain
/// status line may start with a significant space.
pub(crate) fn run_git_capture(workspace: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new(GIT_PROGRAM)
        .args(args)
        .current_dir(workspace)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let s = String::from_utf8_lossy(&output.stdout)
        .trim_end()
        .to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// The real adapter: every method is one `git` spawn.
pub struct GitCli;

impl Git for GitCli {
    fn head(&self, workspace: &Path) -> Option<String> {
        run_git_capture(workspace, &[REV_PARSE, HEAD_REF])
    }

    fn branch(&self, workspace: &Path) -> Option<String> {
        run_git_capture(workspace, &[REV_PARSE, ABBREV_REF_FLAG, HEAD_REF])
    }

    fn log(&self, workspace: &Path, n: usize, since: Option<&str>) -> anyhow::Result<Vec<Commit>> {
        let mut args: Vec<String> = vec![
            LOG.to_string(),
            format!("-n{n}"),
            LOG_FORMAT_FLAG.to_string(),
            NAME_ONLY_FLAG.to_string(),
        ];
        if let Some(s) = since {
            args.push(format!("--since={s}"));
        }
        let output = Command::new(GIT_PROGRAM)
            .args(&args)
            .current_dir(workspace)
            .output()
            .map_err(|e| {
                anyhow::anyhow!("failed to spawn git log in {}: {e}", workspace.display())
            })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!(
                "git log exited with status {} in {}: {}",
                output.status.code().unwrap_or(-1),
                workspace.display(),
                stderr.trim()
            );
        }
        let raw = String::from_utf8_lossy(&output.stdout);
        Ok(crate::gitlog::parse_log(&raw))
    }

    fn changed(&self, workspace: &Path, base: &str) -> Vec<String> {
        crate::gitlog::changed_with(workspace, base, run_git_capture)
    }

    fn last_commit_date(&self, workspace: &Path, file: &str) -> Option<String> {
        let output = Command::new(GIT_PROGRAM)
            .arg(GIT_DIR_FLAG)
            .arg(workspace)
            .arg(LOG)
            .arg(LAST_ONE_FLAG)
            .arg(COMMITTER_DATE_FORMAT_FLAG)
            .arg(PATH_SEPARATOR_ARG)
            .arg(file)
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
        (!text.is_empty()).then_some(text)
    }

    fn is_usable(&self, workspace: &Path) -> bool {
        Command::new(GIT_PROGRAM)
            .arg(GIT_DIR_FLAG)
            .arg(workspace)
            .arg(REV_PARSE)
            .arg(IS_INSIDE_WORK_TREE_FLAG)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn worktree_list(&self, workspace: &Path) -> Vec<PathBuf> {
        let output = match Command::new(GIT_PROGRAM)
            .args([WORKTREE, LIST, PORCELAIN_FLAG])
            .current_dir(workspace)
            .output()
        {
            Ok(o) if o.status.success() => o,
            _ => return Vec::new(),
        };
        let text = String::from_utf8_lossy(&output.stdout);
        let root_canon = workspace
            .canonicalize()
            .unwrap_or_else(|_| workspace.to_path_buf());
        text.lines()
            .filter_map(|line| line.strip_prefix(WORKTREE_LINE_PREFIX))
            .map(|p| PathBuf::from(p.trim()))
            .filter(|p| {
                let canon = p.canonicalize().unwrap_or_else(|_| p.clone());
                canon != root_canon
            })
            .collect()
    }
}

impl Git for RecordingGit {
    fn head(&self, _workspace: &Path) -> Option<String> {
        self.calls.head.fetch_add(1, Ordering::SeqCst);
        self.head_value.clone()
    }

    fn branch(&self, _workspace: &Path) -> Option<String> {
        self.calls.branch.fetch_add(1, Ordering::SeqCst);
        self.branch_value.clone()
    }

    fn log(
        &self,
        _workspace: &Path,
        _n: usize,
        _since: Option<&str>,
    ) -> anyhow::Result<Vec<Commit>> {
        self.calls.log.fetch_add(1, Ordering::SeqCst);
        Ok(self.log_value.clone())
    }

    fn changed(&self, _workspace: &Path, _base: &str) -> Vec<String> {
        self.calls.changed.fetch_add(1, Ordering::SeqCst);
        self.changed_value.clone()
    }

    fn last_commit_date(&self, _workspace: &Path, file: &str) -> Option<String> {
        self.calls.last_commit_date.fetch_add(1, Ordering::SeqCst);
        self.last_commit_dates
            .lock()
            .expect(RECORDING_GIT_POISONED)
            .get(file)
            .cloned()
    }

    fn is_usable(&self, _workspace: &Path) -> bool {
        self.calls.is_usable.fetch_add(1, Ordering::SeqCst);
        self.usable
    }

    fn worktree_list(&self, _workspace: &Path) -> Vec<PathBuf> {
        self.calls.worktree_list.fetch_add(1, Ordering::SeqCst);
        self.worktrees.clone()
    }
}

#[cfg(test)]
#[path = "../tests/unit/git_tests.rs"]
mod tests;
