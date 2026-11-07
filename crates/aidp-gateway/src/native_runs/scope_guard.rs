//! Post-stage write-scope guard: finds files a stage changed outside its scope and restores them.

use crate::models::constants::agent_jobs::HOST_SESSION_DIR;
use crate::models::constants::task_scope::{
    COPY_MARKER, HEAD_REF, NUL_SEPARATED_FLAG, NUL_SEPARATOR, PORCELAIN_FLAG, RENAME_MARKER,
    REV_PARSE_ARG, STATUS_ARG, STATUS_CODE_WIDTH, STATUS_PREFIX_WIDTH, UNTRACKED_ALL_FLAG,
};
use crate::models::scope_baseline::ScopeBaseline;
use crate::models::scope_verdict::ScopeVerdict;
use crate::models::task_scope_verdict::TaskScopeVerdict;
use crate::models::task_write_baseline::TaskWriteBaseline;
use aidp_core::git::Git;
use aidp_core::plan::normalize_plan_path;
use aidp_core::process_runner::{ProcessRunner, ProcessSpec};
use aidp_core::scope::{classify, ScopeRules};
use std::collections::{BTreeMap, BTreeSet};
use std::hash::{Hash, Hasher};
use std::path::Path;

const GIT_PROGRAM: &str = "git";
const CAT_FILE_ARG: &str = "cat-file";
const EXISTS_FLAG: &str = "-e";
const CHECKOUT_ARG: &str = "checkout";
const REMOVE_ARG: &str = "rm";
const FORCE_FLAG: &str = "-f";
const CACHED_FLAG: &str = "--cached";
const QUIET_FLAG: &str = "-q";
const IGNORE_UNMATCH_FLAG: &str = "--ignore-unmatch";
const PATH_SEPARATOR_ARG: &str = "--";
const REVISION_PATH_SEPARATOR: char = ':';
const GIT_PATH_SEPARATOR: char = '/';
const WINDOWS_PATH_SEPARATOR: char = '\\';

/// The run directory as git prints it, when it lives inside the workspace (no vault).
/// A guard leaves it alone: the run writes its own state there.
pub fn run_dir_exemption(workspace: &Path, run_dir: &Path) -> Option<String> {
    let relative = run_dir.strip_prefix(workspace).ok()?;
    let text = relative
        .to_string_lossy()
        .replace(WINDOWS_PATH_SEPARATOR, &GIT_PATH_SEPARATOR.to_string());
    (!text.is_empty()).then_some(text)
}

/// True for the host session folder and for the exempt folder, anything under them, and a folder
/// entry git collapsed around them (`.aidp/` when the whole folder is untracked).
pub(crate) fn is_exempt(path: &str, exempt_prefix: Option<&str>) -> bool {
    is_under(path, HOST_SESSION_DIR) || exempt_prefix.is_some_and(|prefix| is_under(path, prefix))
}

fn is_under(path: &str, prefix: &str) -> bool {
    path == prefix
        || path
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.starts_with(GIT_PATH_SEPARATOR))
        || (path.ends_with(GIT_PATH_SEPARATOR) && prefix.starts_with(path))
}

/// Records HEAD and the paths already dirty, before the stage runs. `exempt_prefix` is left alone.
pub fn snapshot(workspace: &Path, git: &dyn Git, exempt_prefix: Option<&str>) -> ScopeBaseline {
    let head = git.head(workspace);
    let dirty_paths = head
        .as_deref()
        .map(|commit| git.changed(workspace, commit).into_iter().collect())
        .unwrap_or_default();
    ScopeBaseline {
        head,
        dirty_paths,
        exempt_prefix: exempt_prefix.map(str::to_string),
    }
}

/// Restores every file the stage changed outside `stage_rules` and reports them.
/// Without a baseline HEAD nothing can be compared, so the verdict is clean.
pub fn verify(
    baseline: &ScopeBaseline,
    stage_rules: &ScopeRules,
    workspace: &Path,
    git: &dyn Git,
    runner: &dyn ProcessRunner,
) -> anyhow::Result<ScopeVerdict> {
    let Some(head) = baseline.head.as_deref() else {
        return Ok(ScopeVerdict::default());
    };
    let touched: Vec<String> = git
        .changed(workspace, head)
        .into_iter()
        .filter(|path| !baseline.dirty_paths.contains(path))
        .filter(|path| !is_exempt(path, baseline.exempt_prefix.as_deref()))
        .collect();
    let rules = [(String::new(), stage_rules.clone())];
    let mut verdict = ScopeVerdict::default();
    for violation in classify(&touched, &rules) {
        restore_file(workspace, head, &violation.path, runner)?;
        verdict.restored.push(violation.path.clone());
        verdict.violations.push(violation.path);
    }
    Ok(verdict)
}

/// Records HEAD, plus the content and hash of every dirty path, before a plan task runs.
/// `exempt_prefix` is left alone.
pub fn capture_task_baseline(
    worktree: &Path,
    runner: &dyn ProcessRunner,
    exempt_prefix: Option<&str>,
) -> TaskWriteBaseline {
    let contents = dirty_contents(worktree, runner, exempt_prefix);
    let hashes = contents
        .iter()
        .map(|(path, bytes)| (path.clone(), bytes.as_deref().map(hash_bytes)))
        .collect();
    TaskWriteBaseline {
        head: current_head(worktree, runner),
        hashes,
        contents,
        exempt_prefix: exempt_prefix.map(str::to_string),
    }
}

/// Paths whose presence or content differs from `baseline`, as git prints them.
pub fn changed_since(
    worktree: &Path,
    baseline: &TaskWriteBaseline,
    runner: &dyn ProcessRunner,
) -> Vec<String> {
    let current: BTreeMap<String, Option<u64>> =
        dirty_contents(worktree, runner, baseline.exempt_prefix.as_deref())
            .into_iter()
            .map(|(path, bytes)| (path, bytes.as_deref().map(hash_bytes)))
            .collect();
    let mut changed: BTreeSet<String> = current
        .iter()
        .filter(|(path, hash)| baseline.hashes.get(*path) != Some(*hash))
        .map(|(path, _)| path.clone())
        .collect();
    changed.extend(
        baseline
            .hashes
            .keys()
            .filter(|path| !current.contains_key(*path))
            .cloned(),
    );
    changed.into_iter().collect()
}

/// Flags every changed path that is a shared file or is neither a task file nor an allowed extra.
pub fn verify_task_writes(
    changed: &[String],
    task_files: &[String],
    allowed_extra: &[String],
    shared_files: &[String],
) -> TaskScopeVerdict {
    let owned = normalized_set(task_files.iter().chain(allowed_extra));
    let shared = normalized_set(shared_files.iter());
    let violations = changed
        .iter()
        .filter(|path| {
            let key = normalize_plan_path(path);
            shared.contains(&key) || !owned.contains(&key)
        })
        .cloned()
        .collect();
    TaskScopeVerdict { violations }
}

/// Restores only the violating paths, leaving sibling work alone: a path dirty before the task
/// gets its pre-task content back, any other path goes back to the baseline HEAD.
pub fn restore_task_violations(
    worktree: &Path,
    violations: &[String],
    baseline: &TaskWriteBaseline,
    runner: &dyn ProcessRunner,
) -> anyhow::Result<()> {
    for path in violations {
        if let Some(content) = baseline.contents.get(path) {
            restore_content(worktree, path, content.as_deref())?;
        } else if let Some(head) = baseline.head.as_deref() {
            restore_file(worktree, head, path, runner)?;
        }
    }
    Ok(())
}

/// Paths changed by both tasks, as spelled in `first`, for tasks whose windows overlapped.
pub fn overlapping_writes(first: &[String], second: &[String]) -> Vec<String> {
    let other = normalized_set(second.iter());
    first
        .iter()
        .filter(|path| other.contains(&normalize_plan_path(path)))
        .cloned()
        .collect()
}

fn normalized_set<'a>(paths: impl Iterator<Item = &'a String>) -> BTreeSet<String> {
    paths.map(|path| normalize_plan_path(path)).collect()
}

fn current_head(worktree: &Path, runner: &dyn ProcessRunner) -> Option<String> {
    let spec = git_spec(worktree).args([REV_PARSE_ARG, HEAD_REF]);
    let output = runner.output(&spec).ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Bytes of every dirty path; `None` marks a path missing from disk.
fn dirty_contents(
    worktree: &Path,
    runner: &dyn ProcessRunner,
    exempt_prefix: Option<&str>,
) -> BTreeMap<String, Option<Vec<u8>>> {
    dirty_paths(worktree, runner)
        .into_iter()
        .filter(|path| !is_exempt(path, exempt_prefix))
        .map(|path| {
            let bytes = std::fs::read(worktree.join(&path)).ok();
            (path, bytes)
        })
        .collect()
}

/// Writes `content` back to `path`, or deletes the file when it was missing before.
fn restore_content(worktree: &Path, path: &str, content: Option<&[u8]>) -> anyhow::Result<()> {
    let target = worktree.join(path);
    match content {
        Some(bytes) => Ok(std::fs::write(target, bytes)?),
        None => match std::fs::remove_file(target) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.into()),
            _ => Ok(()),
        },
    }
}

fn hash_bytes(bytes: &[u8]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

/// Dirty paths from `git status --porcelain -z`, skipping the origin entry of renames and copies.
fn dirty_paths(worktree: &Path, runner: &dyn ProcessRunner) -> Vec<String> {
    let spec = git_spec(worktree).args([
        STATUS_ARG,
        PORCELAIN_FLAG,
        NUL_SEPARATED_FLAG,
        UNTRACKED_ALL_FLAG,
    ]);
    let Ok(output) = runner.output(&spec) else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    let mut entries = text.split(NUL_SEPARATOR).filter(|entry| !entry.is_empty());
    let mut paths = Vec::new();
    while let Some(entry) = entries.next() {
        let Some(code) = entry.get(..STATUS_CODE_WIDTH) else {
            continue;
        };
        if code.contains(RENAME_MARKER) || code.contains(COPY_MARKER) {
            entries.next();
        }
        if let Some(path) = entry.get(STATUS_PREFIX_WIDTH..) {
            paths.push(path.to_string());
        }
    }
    paths
}

/// Puts `path` back to its baseline content, or deletes it when it did not exist then.
fn restore_file(
    workspace: &Path,
    head: &str,
    path: &str,
    runner: &dyn ProcessRunner,
) -> anyhow::Result<()> {
    if existed_at(workspace, head, path, runner)? {
        let checkout = git_spec(workspace).args([CHECKOUT_ARG, head, PATH_SEPARATOR_ARG, path]);
        return run_checked(runner, &checkout);
    }
    let unindex = git_spec(workspace).args([
        REMOVE_ARG,
        CACHED_FLAG,
        FORCE_FLAG,
        QUIET_FLAG,
        IGNORE_UNMATCH_FLAG,
        PATH_SEPARATOR_ARG,
        path,
    ]);
    run_checked(runner, &unindex)?;
    match std::fs::remove_file(workspace.join(path)) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.into()),
        _ => Ok(()),
    }
}

/// Whether `path` is tracked in the `head` commit.
fn existed_at(
    workspace: &Path,
    head: &str,
    path: &str,
    runner: &dyn ProcessRunner,
) -> anyhow::Result<bool> {
    let object = format!("{head}{REVISION_PATH_SEPARATOR}{path}");
    let spec = git_spec(workspace).args([CAT_FILE_ARG, EXISTS_FLAG, object.as_str()]);
    Ok(runner.output(&spec)?.status.success())
}

fn git_spec(workspace: &Path) -> ProcessSpec {
    ProcessSpec::new(GIT_PROGRAM).cwd(workspace)
}

fn run_checked(runner: &dyn ProcessRunner, spec: &ProcessSpec) -> anyhow::Result<()> {
    let output = runner.output(spec)?;
    if output.status.success() {
        return Ok(());
    }
    anyhow::bail!(
        "{} {} failed: {}",
        spec.program,
        spec.args.join(" "),
        String::from_utf8_lossy(&output.stderr).trim()
    )
}

#[cfg(test)]
#[path = "../../tests/unit/scope_guard_tests.rs"]
mod tests;
