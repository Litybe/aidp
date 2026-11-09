//! GitHub via the `gh` CLI.
//!
//! Two layers:
//!   * typed entry points used directly by skills (`aidp gh ...`);
//!   * `pub fn` helpers that `ci.rs` composes into CI signals, so the watcher
//!     never shells out to `gh` with ad-hoc argument strings.
//!
//! Everything is a lean projection: the `gha-fix` pipeline reads *why* a job
//! failed, not the 40 000-line raw log.

use crate::models::constants::gh_flow as flow;
use crate::models::constants::process::{GH_NOT_FOUND, GH_PROGRAM, GIT_NOT_FOUND, GIT_PROGRAM};
use crate::models::enums::addressed::Addressed;
use crate::models::enums::last_by::LastBy;
use crate::models::raw_thread_comment::RawThreadComment;
use crate::models::raw_thread_node::RawThreadNode;
use crate::models::review_thread::ReviewThread;
use crate::models::thread_comment::ThreadComment;
use serde_json::{json, Value};
use std::cell::RefCell;
use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::NamedTempFile;

const VIEWER_LOGIN_POINTER: &str = "/data/viewer/login";
const REVIEW_THREAD_NODES_POINTER: &str = "/data/repository/pullRequest/reviewThreads/nodes";

thread_local! {
    /// Directory every `git`/`gh` command built in this module runs in, while
    /// an `in_dir` call is active on this thread. `None` means the process cwd.
    static WORKING_DIR: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// Puts the previous working-directory override back when dropped, so
/// `in_dir` restores it on a normal return and on a panic alike.
struct WorkingDirRestore {
    previous: Option<PathBuf>,
}

impl Drop for WorkingDirRestore {
    fn drop(&mut self) {
        let previous = self.previous.take();
        WORKING_DIR.with(|cell| *cell.borrow_mut() = previous);
    }
}

/// Runs `work` with every `git`/`gh` command of this module executing in
/// `dir`, without touching the process-wide cwd. Scoped to the calling thread;
/// the previous override comes back afterwards, even if `work` panics.
pub fn in_dir<R>(dir: &Path, work: impl FnOnce() -> R) -> R {
    let previous = WORKING_DIR.with(|cell| cell.replace(Some(dir.to_path_buf())));
    let _restore = WorkingDirRestore { previous };
    work()
}

/// Points `command` at the directory set by the active `in_dir`, if any.
fn apply_working_dir(command: &mut Command) {
    WORKING_DIR.with(|cell| {
        if let Some(dir) = cell.borrow().as_ref() {
            command.current_dir(dir);
        }
    });
}

/// A `Command` for `gh`, resolved via [`crate::resolve_program`] and scoped to
/// the active [`in_dir`] override. Every `gh` spawn in this module goes
/// through this, so none of them can fall back to a shell.
fn gh_command() -> anyhow::Result<Command> {
    let program =
        crate::resolve_program(GH_PROGRAM).ok_or_else(|| anyhow::anyhow!(GH_NOT_FOUND))?;
    let mut command = Command::new(program);
    apply_working_dir(&mut command);
    Ok(command)
}

/// A `Command` for `git`, resolved via [`crate::resolve_program`] and scoped
/// to the active [`in_dir`] override.
fn git_command() -> anyhow::Result<Command> {
    let program =
        crate::resolve_program(GIT_PROGRAM).ok_or_else(|| anyhow::anyhow!(GIT_NOT_FOUND))?;
    let mut command = Command::new(program);
    apply_working_dir(&mut command);
    Ok(command)
}

// ----------------------------------------------------------------- shell ----

pub(crate) fn gh(args: &[&str], repo: Option<&str>) -> anyhow::Result<String> {
    let mut c = gh_command()?;
    c.args(args);
    if let Some(r) = repo {
        c.args(["--repo", r]);
    }
    let out = c.output().map_err(|e| anyhow::anyhow!("spawn gh: {e}"))?;
    if !out.status.success() {
        anyhow::bail!(
            "gh {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// Like `gh` but returns `None` instead of an error - for probes that are
/// allowed to come up empty (no PR for this branch, no runs yet).
fn gh_opt(args: &[&str], repo: Option<&str>) -> Option<String> {
    gh(args, repo).ok()
}

const MAX_PR_BODY_BYTES: usize = 256 * 1024;

/// `gh pr create`, always via `--body-file` - never `--body` on argv. Writes
/// the body to a private temp file, invokes `gh` directly (this path never
/// goes through the `gh()` helper's repo-flag plumbing, and applies no shell,
/// so no `cmd /C` fallback ever touches this argv), removes the temp file on
/// every exit path, and checks the exit code so a `gh` too old to know
/// `--body-file` fails loudly instead of silently creating a bodyless PR.
pub fn pr_create(title: &str, body: &str, base: Option<&str>) -> anyhow::Result<String> {
    pr_create_draft(title, body, base, false)
}

/// Same as [`pr_create`], adding `--draft` when `draft` is true.
pub fn pr_create_draft(
    title: &str,
    body: &str,
    base: Option<&str>,
    draft: bool,
) -> anyhow::Result<String> {
    pr_create_on(title, body, base, None, draft)
}

/// Same as [`pr_create_draft`] for the pushed branch `head`, which need not be checked out or tracked.
pub fn pr_create_for_head(
    title: &str,
    body: &str,
    head: &str,
    base: Option<&str>,
    draft: bool,
) -> anyhow::Result<String> {
    pr_create_on(title, body, base, Some(head), draft)
}

fn pr_create_on(
    title: &str,
    body: &str,
    base: Option<&str>,
    head: Option<&str>,
    draft: bool,
) -> anyhow::Result<String> {
    if body.len() > MAX_PR_BODY_BYTES {
        anyhow::bail!(
            "PR body is {} bytes, over the {MAX_PR_BODY_BYTES}-byte limit",
            body.len()
        );
    }
    let tmp_path = if body.is_empty() {
        None
    } else {
        let mut f =
            NamedTempFile::new().map_err(|e| anyhow::anyhow!("create PR body temp file: {e}"))?;
        f.write_all(body.as_bytes())
            .map_err(|e| anyhow::anyhow!("write PR body temp file: {e}"))?;
        f.flush()
            .map_err(|e| anyhow::anyhow!("flush PR body temp file: {e}"))?;
        Some(f.into_temp_path())
    };

    let result = (|| -> anyhow::Result<String> {
        let mut c = gh_command()?;
        c.args(["pr", "create", "--title", title]);
        if let Some(p) = &tmp_path {
            c.arg("--body-file");
            c.arg(p.as_os_str());
        }
        if let Some(b) = base {
            c.args(["--base", b]);
        }
        if let Some(branch) = head {
            c.args([flow::HEAD_FLAG, branch]);
        }
        if draft {
            c.arg("--draft");
        }
        let out = c.output().map_err(|e| anyhow::anyhow!("spawn gh: {e}"))?;
        if !out.status.success() {
            anyhow::bail!("gh pr create: {}", String::from_utf8_lossy(&out.stderr));
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    })();

    // tmp_path is automatically deleted when dropped
    result
}

fn json_of(s: &str) -> Value {
    serde_json::from_str(s).unwrap_or(Value::Null)
}

// -------------------------------------------------------------- pr / repo ----

pub fn repo_slug(repo: Option<&str>) -> Option<String> {
    if let Some(r) = repo {
        return Some(r.to_string());
    }
    let out = gh_opt(&["repo", "view", "--json", "nameWithOwner"], None)?;
    json_of(&out)
        .get("nameWithOwner")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

pub fn current_branch() -> Option<String> {
    let mut command = git_command().ok()?;
    let out = command
        .args(["rev-parse", "--abbrev-ref", "HEAD"])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let b = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if b.is_empty() || b == "HEAD" {
        None
    } else {
        Some(b)
    }
}

const PR_FIELDS: &str =
    "number,url,title,state,headRefName,baseRefName,headRefOid,isDraft,mergeable";

/// True when a `gh pr view` failure is the everyday "this branch has no pull
/// request" case, not an actual `gh` malfunction.
fn is_no_pr_found(message: &str) -> bool {
    message.to_lowercase().contains("no pull requests found")
}

/// Resolve the PR in play. `pr` may be a number, a URL, or `None` (current branch).
/// Collapses every `gh` failure into `None` alongside the ordinary "no PR for
/// this branch" case - use [`pr_resolve_checked`] where the two must be told
/// apart.
pub fn pr_resolve(pr: Option<&str>, repo: Option<&str>) -> Option<Value> {
    pr_resolve_checked(pr, repo).ok().flatten()
}

/// Resolve the PR in play, distinguishing "gh ran fine and found nothing" from
/// "gh itself failed" (missing, unauthenticated, rate-limited, non-zero exit).
/// `Ok(None)` is the former; `Err` is the latter, carrying `gh`'s own message.
pub fn pr_resolve_checked(
    pr: Option<&str>,
    repo: Option<&str>,
) -> Result<Option<Value>, crate::IntegrationError> {
    let mut args: Vec<String> = vec!["pr".into(), "view".into()];
    if let Some(p) = pr.filter(|s| !s.is_empty() && !s.contains("{{")) {
        args.push(p.to_string());
    }
    args.push("--json".into());
    args.push(PR_FIELDS.into());
    let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    let out = match gh(&refs, repo) {
        Ok(out) => out,
        Err(e) if is_no_pr_found(&e.to_string()) => return Ok(None),
        Err(e) => return Err(crate::IntegrationError::Upstream(e.to_string())),
    };
    let v = json_of(&out);
    let Some(number) = v.get("number").and_then(|n| n.as_i64()) else {
        return Ok(None);
    };
    Ok(Some(json!({
        "number": number,
        "url": v.get("url"),
        "title": v.get("title"),
        "state": v.get("state"),
        "head_ref": v.get("headRefName"),
        "base_ref": v.get("baseRefName"),
        "head_sha": v.get("headRefOid"),
        "is_draft": v.get("isDraft"),
        "mergeable": v.get("mergeable"),
        "repo": repo_slug(repo),
    })))
}

/// The caller's own open pull requests in this repo, newest first.
///
/// The autopilot needs this because a pull request is only discoverable from its
/// branch, and the branch is routinely not checked out: Claude Code runs isolated
/// stages in throwaway worktrees, and people switch branches. Restricted to
/// `--author @me` on purpose - chasing a colleague's red build is not this
/// tool's business.
pub fn pr_list_mine(repo: Option<&str>, limit: u32) -> Vec<Value> {
    let limit_s = limit.to_string();
    let args = [
        "pr",
        "list",
        "--state",
        "open",
        "--author",
        "@me",
        "--limit",
        &limit_s,
        "--json",
        "number,headRefName,isDraft,updatedAt,title,url",
    ];
    let Some(text) = gh_opt(&args, repo) else {
        return Vec::new();
    };
    let mut arr: Vec<Value> = json_of(&text).as_array().cloned().unwrap_or_default();
    arr.sort_by(|a, b| {
        b.get("updatedAt")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .cmp(a.get("updatedAt").and_then(|v| v.as_str()).unwrap_or(""))
    });
    arr
}

// ---------------------------------------------------------------- checks ----

/// Every check run on the PR, bucketed by `gh pr checks`.
/// `bucket` is one of pass | fail | pending | skipping | cancel.
pub fn pr_checks(pr: Option<&str>, repo: Option<&str>) -> anyhow::Result<Value> {
    let mut args: Vec<String> = vec!["pr".into(), "checks".into()];
    if let Some(p) = pr.filter(|s| !s.is_empty()) {
        args.push(p.to_string());
    }
    args.push("--json".into());
    args.push("name,state,bucket,link,workflow,event,description,completedAt".into());
    let refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
    // `gh pr checks` exits non-zero when any check failed - that is data, not
    // an error, so read stdout regardless.
    let text =
        gh(&refs, repo).unwrap_or_else(|_| gh_opt(&refs, repo).unwrap_or_else(|| "[]".to_string()));
    let arr = json_of(&text);
    let all: Vec<Value> = arr.as_array().cloned().unwrap_or_default();
    let pick = |bucket: &str| -> Vec<Value> {
        all.iter()
            .filter(|c| c.get("bucket").and_then(|b| b.as_str()) == Some(bucket))
            .cloned()
            .collect()
    };
    let failing = {
        let mut v = pick("fail");
        v.extend(pick("cancel"));
        v
    };
    let pending = pick("pending");
    let conclusion = if !failing.is_empty() {
        "failure"
    } else if !pending.is_empty() {
        "pending"
    } else if all.is_empty() {
        "none"
    } else {
        "success"
    };
    Ok(json!({
        "conclusion": conclusion,
        "total": all.len(),
        "failing": failing,
        "pending": pending,
        "all": all,
    }))
}

/// `gh pr checks` output for a check named like Sonar, if the org decorates PRs
/// with one. Gives a gate verdict even with no Sonar token configured.
pub fn sonar_check(checks: &Value) -> Option<Value> {
    let all = checks.get("all")?.as_array()?;
    all.iter()
        .find(|c| {
            let n = c
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_lowercase();
            let w = c
                .get("workflow")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_lowercase();
            n.contains("sonar") || w.contains("sonar") || n.contains("quality gate")
        })
        .cloned()
}

// ----------------------------------------------------------- review threads ----

const THREADS_QUERY: &str = r#"
query($owner:String!,$name:String!,$number:Int!){
  viewer{ login }
  repository(owner:$owner,name:$name){
    pullRequest(number:$number){
      reviewThreads(first:100){
        nodes{
          id isResolved isOutdated path line startLine
          comments(first:50){ totalCount nodes{ id databaseId author{login} body createdAt url diffHunk } }
        }
      }
    }
  }
}"#;

const MAX_THREAD_COMMENTS: i64 = 50;

/// Lists review threads with no thread reported as indirectly addressed.
pub fn pr_review_threads(
    pr: Option<&str>,
    repo: Option<&str>,
    include_resolved: bool,
) -> anyhow::Result<Value> {
    pr_review_threads_with_handed(pr, repo, include_resolved, &HashSet::new())
}

/// Lists review threads; thread ids in `handed` are reported as indirectly addressed.
pub fn pr_review_threads_with_handed(
    pr: Option<&str>,
    repo: Option<&str>,
    include_resolved: bool,
    handed: &HashSet<String>,
) -> anyhow::Result<Value> {
    let resolved = pr_resolve(pr, repo)
        .ok_or_else(|| anyhow::anyhow!("no pull request for this branch (and none given)"))?;
    let number = resolved.get("number").and_then(|n| n.as_i64()).unwrap_or(0);
    let slug = resolved
        .get("repo")
        .and_then(|r| r.as_str())
        .ok_or_else(|| anyhow::anyhow!("cannot resolve owner/repo"))?
        .to_string();
    let (owner, name) = slug
        .split_once('/')
        .ok_or_else(|| anyhow::anyhow!("bad repo slug {slug}"))?;

    let num = number.to_string();
    let text = gh(
        &[
            "api",
            "graphql",
            "-f",
            &format!("query={THREADS_QUERY}"),
            "-f",
            &format!("owner={owner}"),
            "-f",
            &format!("name={name}"),
            "-F",
            &format!("number={num}"),
        ],
        None,
    )?;
    let v = json_of(&text);
    let viewer = v
        .pointer(VIEWER_LOGIN_POINTER)
        .and_then(|login| login.as_str())
        .unwrap_or_default();
    let nodes = v
        .pointer(REVIEW_THREAD_NODES_POINTER)
        .and_then(|n| n.as_array())
        .cloned()
        .unwrap_or_default();

    let all_threads: Vec<ReviewThread> = nodes
        .iter()
        .map(|node| derive_review_thread(node, viewer, handed))
        .collect();
    let unresolved = all_threads.iter().filter(|t| !t.is_resolved).count();
    let threads: Vec<ReviewThread> = all_threads
        .into_iter()
        .filter(|thread| include_resolved || !thread.is_resolved)
        .collect();
    Ok(json!({
        "pr": number,
        "repo": slug,
        "total": nodes.len(),
        "unresolved": unresolved,
        "threads": threads,
    }))
}

/// Builds a thread from a raw GraphQL node, deriving `lastBy`, `addressed` and
/// `waitingOnReviewer` without any gh call.
pub fn derive_review_thread(node: &Value, viewer: &str, handed: &HashSet<String>) -> ReviewThread {
    let raw: RawThreadNode = serde_json::from_value(node.clone()).unwrap_or_default();
    let total = raw.comments.total_count;
    let comments: Vec<ThreadComment> = raw
        .comments
        .nodes
        .into_iter()
        .map(|comment| thread_comment(comment, viewer))
        .collect();
    let is_resolved = raw.is_resolved;
    let is_outdated = raw.is_outdated;
    let thread_id = raw.id;

    let last_by = match comments.last() {
        Some(last) if last.is_mine => LastBy::Me,
        _ => LastBy::Other,
    };
    let addressed = if is_outdated == Some(true) {
        Addressed::Direct
    } else if thread_id.as_ref().is_some_and(|id| handed.contains(id)) {
        Addressed::Indirect
    } else {
        Addressed::No
    };
    let first = comments.first();
    ReviewThread {
        thread_id,
        reply_to_id: first.and_then(|c| c.id.clone()),
        is_resolved,
        is_outdated,
        path: raw.path,
        line: raw.line,
        author: first.and_then(|c| c.author.clone()),
        created_at: first.and_then(|c| c.created_at.clone()),
        body: first.and_then(|c| c.body.clone()),
        start_line: raw.start_line,
        comments_truncated: total > MAX_THREAD_COMMENTS,
        waiting_on_reviewer: !is_resolved && last_by == LastBy::Me,
        last_by,
        addressed,
        comments,
    }
}

fn thread_comment(raw: RawThreadComment, viewer: &str) -> ThreadComment {
    let author = raw.author.and_then(|author| author.login);
    ThreadComment {
        id: raw.id,
        database_id: raw.database_id,
        is_mine: !viewer.is_empty() && author.as_deref() == Some(viewer),
        author,
        body: raw.body,
        created_at: raw.created_at,
        url: raw.url,
        diff_hunk: raw.diff_hunk,
    }
}

// ------------------------------------------------------------------- runs ----

/// Runs `gh` with `args` and returns its stdout, failing with `label` and stderr.
pub(crate) fn gh_stdout(label: &str, args: &[&str]) -> anyhow::Result<String> {
    let mut command = gh_command()?;
    let out = command.args(args).output()?;
    if !out.status.success() {
        anyhow::bail!("{label}: {}", String::from_utf8_lossy(&out.stderr));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// Raw `gh pr view --json` projection of a PR.
pub fn pr_view(pr: &str) -> anyhow::Result<String> {
    gh_stdout(
        "gh pr view",
        &[
            "pr",
            "view",
            pr,
            "--json",
            "number,url,title,state,mergeable",
        ],
    )
}

/// Raw `gh pr diff` text of a PR.
pub fn pr_diff(pr: &str) -> anyhow::Result<String> {
    gh_stdout("gh pr diff", &["pr", "diff", pr])
}

/// The PR in play, or `{found:false}` when none resolves.
pub fn pr_resolve_or_missing(pr: Option<&str>, repo: Option<&str>) -> Value {
    pr_resolve(pr, repo).unwrap_or_else(|| json!({"found": false}))
}

#[cfg(test)]
#[path = "../tests/unit/gh_tests.rs"]
mod gh_tests;

#[cfg(test)]
#[path = "../tests/unit/review_threads_tests.rs"]
mod review_threads_tests;
