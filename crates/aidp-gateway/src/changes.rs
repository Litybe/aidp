//! Lot 2a: git/gh session changes for `GET /api/sessions/:id/changes`.
//!
//! Pure/impure split, mirroring `aidp-core::gitlog` (see that module's header
//! for the rationale): pure functions are plain functions of already-in-memory
//! text, fully unit-testable from fixtures; impure functions shell out via
//! `std::process::Command` and degrade to `None`/empty on any failure (git or
//! gh absent, not a repository, bad ref) rather than propagating an error or
//! panicking — this endpoint must answer even in a directory that is not a
//! git repository.
//!
//! `aidp-core` is deliberately NOT a dependency here: `gitlog::changed()`
//! returns bare paths with no per-file status letter or insertion/deletion
//! counts, which is exactly what this endpoint must report, and its
//! `run_git` helper is private. A local `run_git` is duplicated instead.
//! `aidp-integrations::gh` is deliberately left untouched too: its `gh()`
//! helper never sets `.current_dir(...)`, and the daemon serves N sessions
//! across N projects, so this module shells out to `gh` locally with
//! `.current_dir(cwd)` instead of widening that shared helper's signature.

use crate::models::constants::process::{CAT_FILE, EXISTS_FLAG, GH_PROGRAM, GIT_PROGRAM};
use aidp_core::process_runner::{OsProcessRunner, ProcessRunner, ProcessSpec};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

/// One changed file as reported by `git status --porcelain` merged with
/// `git diff --stat`.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ChangedFile {
    pub path: String,
    /// The two raw porcelain-v1 status characters, e.g. " M", "A ", "??", "R ".
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub orig_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub insertions: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deletions: Option<u32>,
}

fn normalize_path(p: &str) -> String {
    p.replace('\\', "/")
}

/// PURE: parse `git status --porcelain` output.
///
/// THE ONE RULE: never `.trim()` / `.trim_start()` a porcelain line — the
/// first of the two status characters is a significant space (` M` = modified
/// in the worktree but not staged, `M ` = staged). Only `trim_end()` is used
/// here; the status is taken as bytes 0..2, byte 2 (the separator space) is
/// skipped, and the path starts at byte 3. See
/// `aidp-core/src/gitlog.rs`'s `changed_does_not_corrupt_the_alphabetically_first_status_line`
/// for the regression this guards against.
pub fn parse_status(raw: &str) -> Vec<ChangedFile> {
    let mut out = Vec::new();
    for raw_line in raw.split('\n') {
        let line = raw_line.trim_end_matches('\r');
        if line.len() < 4 {
            continue;
        }
        let bytes = line.as_bytes();
        let status = format!("{}{}", bytes[0] as char, bytes[1] as char);
        // byte 2 is the separator space; the path starts at byte 3.
        let rest = &line[3..];
        let (path_part, orig) = match rest.find(" -> ") {
            Some(idx) => (&rest[idx + 4..], Some(&rest[..idx])),
            None => (rest, None),
        };
        let unquote = |s: &str| s.trim_matches('"').to_string();
        let path = normalize_path(&unquote(path_part));
        if path.is_empty() {
            continue;
        }
        out.push(ChangedFile {
            path,
            status,
            orig_path: orig.map(|o| normalize_path(&unquote(o))),
            insertions: None,
            deletions: None,
        });
    }
    out
}

/// PURE: parse `git diff --stat` output into `(path, insertions, deletions)`.
///
/// Lines look like ` path/to/file.rs | 12 +++++-----`, terminated by a
/// ` N files changed, ...` summary line that is ignored. Insertions/deletions
/// are counted from the `+`/`-` characters; a binary file's line (e.g.
/// `Bin 0 -> 12 bytes`) has none and yields `(0, 0)`. A rename renders as
/// `old => new` inside the path column and is normalized to the new path.
/// Any line without a `|` is ignored.
pub fn parse_diff_stat(raw: &str) -> Vec<(String, u32, u32)> {
    let mut out = Vec::new();
    for raw_line in raw.split('\n') {
        let line = raw_line.trim_end_matches('\r');
        let Some(bar_idx) = line.find('|') else {
            continue;
        };
        let path_col = line[..bar_idx].trim();
        if path_col.is_empty() {
            continue;
        }
        let path = match path_col.find(" => ") {
            Some(idx) => &path_col[idx + 4..],
            None => path_col,
        };
        let rest = &line[bar_idx + 1..];
        let insertions = rest.chars().filter(|c| *c == '+').count() as u32;
        let deletions = rest.chars().filter(|c| *c == '-').count() as u32;
        out.push((normalize_path(path.trim()), insertions, deletions));
    }
    out
}

/// PURE: join `status` (source of truth for which files appear — it alone
/// sees untracked files) with `stat` (fills in `insertions`/`deletions` when
/// available). A file present only in `stat` (committed since the base sha
/// but clean in the worktree) still appears, with status `"  "` (two
/// spaces), letting the caller tell it apart from a worktree change.
pub fn merge(status: Vec<ChangedFile>, stat: Vec<(String, u32, u32)>) -> Vec<ChangedFile> {
    let mut out: Vec<ChangedFile> = status;

    for (path, ins, del) in stat {
        if let Some(existing) = out.iter_mut().find(|f| f.path == path) {
            existing.insertions = Some(ins);
            existing.deletions = Some(del);
        } else {
            out.push(ChangedFile {
                path,
                status: "  ".to_string(),
                orig_path: None,
                insertions: Some(ins),
                deletions: Some(del),
            });
        }
    }

    out
}

/// Run `git <args>` in `cwd`, returning trimmed (trailing only) stdout on
/// success. `None` on any failure (git absent, non-zero exit, not a
/// repository, empty output) — every impure function in this module is built
/// from this primitive and inherits its non-failure contract.
fn run_git(cwd: &Path, args: &[&str]) -> Option<String> {
    let spec = ProcessSpec::new(GIT_PROGRAM)
        .args(args.iter().map(|a| a.to_string()))
        .cwd(cwd.to_path_buf());
    let output = OsProcessRunner.output(&spec).ok()?;
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

/// `git rev-parse HEAD`, i.e. `startSha`.
pub fn head(cwd: &Path) -> Option<String> {
    run_git(cwd, &["rev-parse", "HEAD"])
}

/// `git rev-parse --abbrev-ref HEAD`.
pub fn current_branch(cwd: &Path) -> Option<String> {
    run_git(cwd, &["rev-parse", "--abbrev-ref", "HEAD"])
}

/// `parse_status(git status --porcelain)`. Empty (never an error) when the
/// command fails, e.g. `cwd` is not a git repository.
pub fn status(cwd: &Path) -> Vec<ChangedFile> {
    match run_git(cwd, &["status", "--porcelain"]) {
        Some(raw) => parse_status(&raw),
        None => Vec::new(),
    }
}

/// `parse_diff_stat(git diff --stat <base>)`. Returns `None` (distinct from
/// `Some(vec![])`) when `base` cannot be resolved — that distinction lets the
/// route report a stale `startSha` instead of an empty diff.
pub fn diff_stat(cwd: &Path, base: &str) -> Option<Vec<(String, u32, u32)>> {
    run_git(cwd, &["diff", "--stat", base]).map(|raw| parse_diff_stat(&raw))
}

/// `git diff <base> -- <file>`, for the caller-supplied `?file=` value. The
/// path is passed after a literal `--` separator so a value starting with
/// `-` can never be read as a flag. Values containing `..`, a NUL byte, or
/// that are absolute are rejected (`None`) without ever reaching git — git
/// already confines a relative path to the repository, so no canonicalize is
/// attempted here.
pub fn file_diff(cwd: &Path, base: &str, file: &str) -> Option<String> {
    if file.is_empty()
        || file.contains("..")
        || file.contains('\0')
        || Path::new(file).is_absolute()
    {
        return None;
    }
    run_git(cwd, &["diff", base, "--", file])
}

/// `git cat-file -e <sha>^{commit}`: whether `sha` resolves to a commit in
/// this repository.
pub fn sha_exists(cwd: &Path, sha: &str) -> bool {
    let arg = format!("{sha}^{{commit}}");
    let spec = ProcessSpec::new(GIT_PROGRAM)
        .args([CAT_FILE.to_string(), EXISTS_FLAG.to_string(), arg])
        .cwd(cwd.to_path_buf());
    OsProcessRunner
        .output(&spec)
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// `status(cwd).len()`, for the bus event.
pub fn changed_count(cwd: &Path) -> usize {
    status(cwd).len()
}

/// The `--json` field list shared by `pr()` and `pr_for_branch()`.
const PR_JSON_FIELDS: &str = "number,title,state,url,isDraft,statusCheckRollup";

/// PURE: the `gh pr view [<branch>] --json <fields>` argument vector.
/// Extracted out of `pr_view` so the branch-selection logic (checked-out PR
/// vs. a named branch's PR) is unit-testable without shelling out to `gh`.
fn pr_view_args(branch: Option<&str>) -> Vec<&str> {
    let mut args = vec!["pr", "view"];
    if let Some(b) = branch {
        args.push(b);
    }
    args.push("--json");
    args.push(PR_JSON_FIELDS);
    args
}

/// Shared impure body of `pr()` and `pr_for_branch()`: run
/// `gh pr view [<branch>] --json <fields>` in `cwd`, shelled out locally
/// with `.current_dir(cwd)` (unlike `aidp-integrations::gh::gh()`, which
/// never sets a working directory and would answer for whichever repo the
/// daemon itself happens to be in). Returns `None` when `gh` is missing,
/// unauthenticated, or the target branch has no PR (`gh` exiting non-zero is
/// the normal no-PR case, not an error to surface). The raw
/// `statusCheckRollup` is kept as-is; the caller is responsible for
/// summarizing it.
///
/// This function shells out to a network-backed CLI and must NEVER be called
/// on a per-request or per-tick path — only on a session-ended event or on
/// the periodic merge probe.
fn pr_view(cwd: &Path, branch: Option<&str>) -> Option<Value> {
    let spec = ProcessSpec::new(GH_PROGRAM)
        .args(pr_view_args(branch).into_iter().map(|a| a.to_string()))
        .cwd(cwd.to_path_buf());
    let output = OsProcessRunner.output(&spec).ok()?;
    if !output.status.success() {
        return None;
    }
    serde_json::from_slice(&output.stdout).ok()
}

/// PR of the checked-out branch in `cwd`; network-backed, so never per request
/// except through the `prs.rs` TTL cache. See `pr_view` for the failure posture.
pub fn pr(cwd: &Path) -> Option<Value> {
    pr_view(cwd, None)
}

/// Like `pr()`, but asks about `branch` instead of whatever is checked out —
/// `gh pr view <branch> --json …` run in `cwd`, without checking `branch`
/// out. Same `None`-on-failure posture as `pr()`. See `pr_view` for the cost
/// warning: this shells out to a network-backed CLI and must NEVER be called
/// on a per-request or per-tick path — only on a session-ended event or on
/// the periodic merge probe.
pub fn pr_for_branch(cwd: &Path, branch: &str) -> Option<Value> {
    pr_view(cwd, Some(branch))
}

/// Assemble the `GET /sessions/:id/changes` JSON body (the non-`?file=`
/// branch): resolves `baseShaMissing` from a single `diff_stat` call (no
/// separate `sha_exists` round trip — `diff_stat` already returns `None`
/// when `base` doesn't resolve, per its own doc comment), merges worktree
/// status with the diff stat, sums insertions/deletions, and optionally
/// attaches the current PR. `baseShaMissing` is only ever `true` when
/// `start_sha` is `Some` and it failed to resolve — a session that never had
/// a `start_sha` simply omits the key, since there is nothing to diff
/// against and that is not itself a missing/stale base.
pub fn build_response(
    cwd: &Path,
    session_id: &str,
    cwd_str: &str,
    start_sha: Option<&str>,
    want_pr: bool,
) -> Value {
    let files_status = status(cwd);
    let stat = start_sha.and_then(|sha| diff_stat(cwd, sha));
    let base_sha_missing = start_sha.is_some() && stat.is_none();
    let files = match stat {
        Some(s) => merge(files_status, s),
        None => files_status,
    };
    let insertions: u32 = files.iter().filter_map(|f| f.insertions).sum();
    let deletions: u32 = files.iter().filter_map(|f| f.deletions).sum();

    let mut obj = serde_json::Map::new();
    obj.insert(
        "sessionId".to_string(),
        Value::String(session_id.to_string()),
    );
    obj.insert("cwd".to_string(), Value::String(cwd_str.to_string()));
    if let Some(sha) = start_sha {
        obj.insert("startSha".to_string(), Value::String(sha.to_string()));
    }
    if base_sha_missing {
        obj.insert("baseShaMissing".to_string(), Value::Bool(true));
    }
    if let Some(branch) = current_branch(cwd) {
        obj.insert("branch".to_string(), Value::String(branch));
    }
    obj.insert("files".to_string(), serde_json::to_value(files).unwrap());
    obj.insert("insertions".to_string(), Value::from(insertions));
    obj.insert("deletions".to_string(), Value::from(deletions));

    if want_pr {
        if let Some(pr_value) = pr(cwd) {
            obj.insert("pr".to_string(), pr_value);
        }
    }

    Value::Object(obj)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aidp_testkit::TempWorkspace;
    use std::process::Command as StdCommand;

    fn tmp_repo(label: &str) -> TempWorkspace {
        let workspace = TempWorkspace::new(&format!("changes-{label}"));
        let dir = workspace.path();
        let run = |args: &[&str]| {
            let st = StdCommand::new("git")
                .args(args)
                .current_dir(dir)
                .status()
                .expect("git must be on PATH for this test");
            assert!(st.success(), "git {:?} failed", args);
        };
        run(&["init", "-q"]);
        run(&["config", "user.email", "test@example.com"]);
        run(&["config", "user.name", "aidp tests"]);
        workspace
    }

    // --- pure: parse_status ----------------------------------------------

    #[test]
    fn parse_status_keeps_the_leading_significant_space() {
        let raw = " M src/a.rs\nM  src/b.rs\nA  src/c.rs\n?? src/d.rs\nD  src/e.rs\n";
        let files = parse_status(raw);
        assert_eq!(files.len(), 5);
        assert_eq!(files[0].status, " M");
        assert_eq!(files[0].path, "src/a.rs");
        assert_eq!(files[1].status, "M ");
        assert_eq!(files[1].path, "src/b.rs");
        assert_eq!(files[2].status, "A ");
        assert_eq!(files[2].path, "src/c.rs");
        assert_eq!(files[3].status, "??");
        assert_eq!(files[3].path, "src/d.rs");
        assert_eq!(files[4].status, "D ");
        assert_eq!(files[4].path, "src/e.rs");
    }

    #[test]
    fn parse_status_handles_quoted_path_and_rename() {
        let raw = "M  \"src/quoted file.rs\"\nR  old.rs -> new.rs\n";
        let files = parse_status(raw);
        assert_eq!(files.len(), 2);
        assert_eq!(files[0].path, "src/quoted file.rs");
        assert_eq!(files[1].status, "R ");
        assert_eq!(files[1].path, "new.rs");
        assert_eq!(files[1].orig_path.as_deref(), Some("old.rs"));
    }

    #[test]
    fn parse_status_skips_short_lines_and_empty_input_is_empty() {
        assert!(parse_status("").is_empty());
        let raw = "ab\n M ok.rs\n";
        let files = parse_status(raw);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "ok.rs");
    }

    // --- pure: parse_diff_stat --------------------------------------------

    #[test]
    fn parse_diff_stat_handles_binary_rename_and_summary() {
        let raw = " src/a.rs        | 12 +++++-----\n\
 old.rs => new.rs | 4 ++--\n\
 assets/img.png (Bin 0 -> 12 bytes)  |Bin\n\
 3 files changed, 16 insertions(+), 7 deletions(-)\n";
        let stats = parse_diff_stat(raw);
        // The summary line has no `|`... actually it doesn't contain '|' so
        // it's skipped automatically; assert the three real entries parsed.
        assert_eq!(stats.len(), 3);
        assert_eq!(stats[0].0, "src/a.rs");
        assert_eq!(stats[0].1, 5);
        assert_eq!(stats[0].2, 5);
        assert_eq!(stats[1].0, "new.rs");
        assert_eq!(stats[1].1, 2);
        assert_eq!(stats[1].2, 2);
        assert_eq!(stats[2].0, "assets/img.png (Bin 0 -> 12 bytes)");
        assert_eq!(stats[2].1, 0);
        assert_eq!(stats[2].2, 0);
    }

    // --- pure: merge --------------------------------------------------------

    #[test]
    fn merge_fills_stats_keeps_untracked_and_surfaces_stat_only() {
        let status = vec![
            ChangedFile {
                path: "a.rs".to_string(),
                status: " M".to_string(),
                orig_path: None,
                insertions: None,
                deletions: None,
            },
            ChangedFile {
                path: "untracked.rs".to_string(),
                status: "??".to_string(),
                orig_path: None,
                insertions: None,
                deletions: None,
            },
        ];
        let stat = vec![
            ("a.rs".to_string(), 3, 1),
            ("committed_clean.rs".to_string(), 5, 2),
        ];
        let merged = merge(status, stat);
        assert_eq!(merged.len(), 3);
        let a = merged.iter().find(|f| f.path == "a.rs").unwrap();
        assert_eq!(a.insertions, Some(3));
        assert_eq!(a.deletions, Some(1));
        let untracked = merged.iter().find(|f| f.path == "untracked.rs").unwrap();
        assert_eq!(untracked.status, "??");
        assert!(untracked.insertions.is_none());
        let committed = merged
            .iter()
            .find(|f| f.path == "committed_clean.rs")
            .unwrap();
        assert_eq!(committed.status, "  ");
        assert_eq!(committed.insertions, Some(5));
        assert_eq!(committed.deletions, Some(2));
    }

    // --- pure: pr_view_args ---------------------------------------------------

    #[test]
    fn pr_view_args_targets_the_named_branch_without_checking_it_out() {
        assert_eq!(
            pr_view_args(None),
            vec!["pr", "view", "--json", PR_JSON_FIELDS]
        );
        assert_eq!(
            pr_view_args(Some("feat/lot-9a")),
            vec!["pr", "view", "feat/lot-9a", "--json", PR_JSON_FIELDS]
        );
    }

    // --- serde round-trip ----------------------------------------------------

    #[test]
    fn changed_file_serde_round_trip_is_camel_case_and_omits_none() {
        let f = ChangedFile {
            path: "a.rs".to_string(),
            status: " M".to_string(),
            orig_path: None,
            insertions: Some(3),
            deletions: None,
        };
        let json = serde_json::to_value(&f).unwrap();
        assert_eq!(json["path"], "a.rs");
        assert_eq!(json["status"], " M");
        assert!(json.get("origPath").is_none());
        assert_eq!(json["insertions"], 3);
        assert!(json.get("deletions").is_none());

        let back: ChangedFile = serde_json::from_value(json).unwrap();
        assert_eq!(back.path, "a.rs");
        assert_eq!(back.insertions, Some(3));
        assert!(back.deletions.is_none());
    }

    // --- impure: against a real temp repository ------------------------------

    #[test]
    fn it_impure_functions_round_trip_on_a_real_repo_and_degrade_outside_one() {
        let _ws = tmp_repo("roundtrip");
        let dir = _ws.path();
        std::fs::write(dir.join("tracked.rs"), "one\n").unwrap();
        StdCommand::new("git")
            .args(["add", "."])
            .current_dir(dir)
            .status()
            .unwrap();
        StdCommand::new("git")
            .args(["commit", "-q", "-m", "feat: base"])
            .current_dir(dir)
            .status()
            .unwrap();

        let sha = head(dir).expect("HEAD must resolve on a repo with a commit");
        assert_eq!(sha.len(), 40);
        assert!(current_branch(dir).is_some());
        assert!(sha_exists(dir, &sha));
        assert!(!sha_exists(dir, "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef"));

        // Modify the tracked file and add an untracked one.
        std::fs::write(dir.join("tracked.rs"), "one\ntwo\n").unwrap();
        std::fs::write(dir.join("untracked.rs"), "new\n").unwrap();

        let files = status(dir);
        assert!(files
            .iter()
            .any(|f| f.path == "tracked.rs" && f.status == " M"));
        assert!(files
            .iter()
            .any(|f| f.path == "untracked.rs" && f.status == "??"));
        assert_eq!(changed_count(dir), files.len());

        // diff_stat against the real base sha succeeds; against a fabricated
        // sha it degrades to None.
        assert!(diff_stat(dir, &sha).is_some());
        assert!(diff_stat(dir, "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef").is_none());

        // Outside a git repository entirely, everything degrades.
        let _norepo = TempWorkspace::new("changes-norepo");
        let norepo = _norepo.path();
        assert!(head(norepo).is_none());
        assert!(current_branch(norepo).is_none());
        assert!(status(norepo).is_empty());
        assert!(diff_stat(norepo, "HEAD").is_none());
        assert_eq!(changed_count(norepo), 0);
        assert!(!sha_exists(norepo, "HEAD"));
    }

    // --- build_response ------------------------------------------------------

    #[test]
    fn it_build_response_omits_base_sha_missing_when_no_start_sha() {
        let _ws = tmp_repo("build-response-no-start-sha");
        let dir = _ws.path();
        let value = build_response(dir, "sess-1", &dir.display().to_string(), None, false);
        assert!(value.get("baseShaMissing").is_none());
        assert!(value.get("startSha").is_none());
    }

    #[test]
    fn it_build_response_sets_base_sha_missing_when_start_sha_does_not_resolve() {
        let _ws = tmp_repo("build-response-stale-sha");
        let dir = _ws.path();
        std::fs::write(dir.join("tracked.rs"), "one\n").unwrap();
        StdCommand::new("git")
            .args(["add", "."])
            .current_dir(dir)
            .status()
            .unwrap();
        StdCommand::new("git")
            .args(["commit", "-q", "-m", "feat: base"])
            .current_dir(dir)
            .status()
            .unwrap();
        let stale = "deadbeefdeadbeefdeadbeefdeadbeefdeadbeef";
        let value = build_response(
            dir,
            "sess-2",
            &dir.display().to_string(),
            Some(stale),
            false,
        );
        assert_eq!(value["baseShaMissing"], true);
        assert_eq!(value["startSha"], stale);
    }

    #[test]
    fn it_build_response_merges_diff_and_sums_insertions_deletions_for_a_resolved_sha() {
        let _ws = tmp_repo("build-response-resolved-sha");
        let dir = _ws.path();
        std::fs::write(dir.join("tracked.rs"), "one\n").unwrap();
        StdCommand::new("git")
            .args(["add", "."])
            .current_dir(dir)
            .status()
            .unwrap();
        StdCommand::new("git")
            .args(["commit", "-q", "-m", "feat: base"])
            .current_dir(dir)
            .status()
            .unwrap();
        let sha = head(dir).unwrap();

        std::fs::write(dir.join("tracked.rs"), "one\ntwo\nthree\n").unwrap();
        std::fs::write(dir.join("untracked.rs"), "new\n").unwrap();

        let value = build_response(dir, "sess-3", &dir.display().to_string(), Some(&sha), false);
        assert!(value.get("baseShaMissing").is_none());
        assert_eq!(value["startSha"], sha);
        let files = value["files"].as_array().unwrap();
        assert!(files.iter().any(|f| f["path"] == "tracked.rs"));
        assert!(files.iter().any(|f| f["path"] == "untracked.rs"));
        assert!(value["insertions"].as_u64().unwrap() >= 2);
    }

    #[test]
    fn it_file_diff_rejects_dangerous_values() {
        let _ws = tmp_repo("filediff-reject");
        let dir = _ws.path();
        assert!(file_diff(dir, "HEAD", "../escape.rs").is_none());
        assert!(file_diff(dir, "HEAD", "/abs/path.rs").is_none());
        assert!(file_diff(dir, "HEAD", "with\0nul").is_none());
        assert!(file_diff(dir, "HEAD", "").is_none());
    }
}
