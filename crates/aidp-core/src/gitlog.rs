//! C7 (Lot 3, Capitalisation): git history as a differed signal for `reconcile`.
//!
//! `run::reconcile` marks a stage `completed` the moment its sub-agent's reply
//! envelope says so. That verdict is necessarily premature — nothing in the
//! reply envelope can know whether the commit it produced survives review, gets
//! hotfixed three hours later, or is reverted outright. This module supplies
//! that later verdict from the one source that actually has it: git history
//! itself.
//!
//! Two families of functions, deliberately split so the decision logic is
//! testable without a repository on disk:
//!   * **pure** — [`parse_log`] and [`verdicts`] are plain functions of data
//!     already in memory. No I/O, no `Path`, fully covered by fixture-driven
//!     unit tests (`tests/fixtures/gitlog/log-name-only.txt`).
//!   * **impure** — [`log`], [`head`], [`branch`] and [`changed`] delegate to
//!     [`crate::git::GitCli`] and degrade to an empty/`None` result on any
//!     failure instead of aborting the run.
//!
//! Contract end to end: nothing here ever aborts a run. A repo-less workspace,
//! a missing `git` binary, or a base ref that no longer exists all produce
//! empty/`None` results, which `run::init`/`run::reconcile` treat as "nothing
//! to anchor" and "verification skipped" respectively — never as a failure.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

/// One commit as parsed from `git log`. `files` is the set of paths that
/// commit's tree changed relative to its first parent (`--name-only`).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Commit {
    pub sha: String,
    pub subject: String,
    pub files: BTreeSet<String>,
    pub ts: String,
}

/// The later fate of a commit, as read back from history after the fact.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum Verdict {
    /// No later commit (within the lookback window) reverted it or looks like
    /// a fix targeting the same files.
    Success,
    /// A later commit within the window matches a fix/hotfix/patch keyword and
    /// shares at least `overlap` of its files (of the smaller of the two file
    /// sets — see [`verdicts`]).
    Hotfixed { by: String, overlap: f32 },
    /// A later commit's subject is exactly `Revert "<this commit's subject>"`.
    Reverted { by: String },
}

/// PURE: parse the raw output of
/// `git log --format=%H%x1f%s%x1f%aI --name-only`.
///
/// Robust to the exact blank-line placement `git` uses between entries (it can
/// vary by version and by whether a commit touched zero files): a line is
/// treated as a new commit header the moment it splits into exactly three
/// parts on the `\x1f` (0x1F, ASCII Unit Separator) field separator and the
/// first part looks like a hex object id; every other non-blank line is a file
/// path belonging to whichever commit header came before it. Blank lines are
/// pure separators and never produce empty entries in `Commit::files`.
pub fn parse_log(raw: &str) -> Vec<Commit> {
    let mut commits = Vec::new();
    let mut current: Option<Commit> = None;

    for raw_line in raw.split('\n') {
        let line = raw_line.trim_end_matches('\r');
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split('\u{1f}').collect();
        if parts.len() == 3 && looks_like_object_id(parts[0]) {
            if let Some(c) = current.take() {
                commits.push(c);
            }
            current = Some(Commit {
                sha: parts[0].to_string(),
                subject: parts[1].to_string(),
                files: BTreeSet::new(),
                ts: parts[2].to_string(),
            });
        } else if let Some(c) = current.as_mut() {
            c.files.insert(line.to_string());
        }
        // A stray file-looking line before any header is discarded: it cannot
        // belong to a commit we have not seen yet.
    }
    if let Some(c) = current.take() {
        commits.push(c);
    }
    commits
}

/// Whether `s` looks like a git object id (sha-1 or sha-256): non-empty and
/// entirely lowercase-or-digit hex. Guards [`parse_log`] against ever mistaking
/// a file path for a commit header on the (extremely unlikely) chance a path
/// segment itself contains a literal `\x1f`.
fn looks_like_object_id(s: &str) -> bool {
    !s.is_empty() && s.len() >= 7 && s.chars().all(|c| c.is_ascii_hexdigit())
}

/// `git log -n<n> [--since=<since>] --format=%H%x1f%s%x1f%aI --name-only`,
/// parsed via [`parse_log`]. Errors (git absent, `workspace` not a repository,
/// a malformed `since`) propagate to the caller — unlike [`head`]/[`changed`],
/// which are meant to be sprinkled through best-effort call sites, `log` is the
/// one function here a caller may legitimately want to know failed.
pub fn log(workspace: &Path, n: usize, since: Option<&str>) -> anyhow::Result<Vec<Commit>> {
    use crate::git::Git;
    crate::git::GitCli.log(workspace, n, since)
}

/// `git rev-parse HEAD`. `None` on any failure — the caller (`run::init`)
/// treats a `None` as "nothing to anchor", not as an error: a workspace with no
/// git repository must still be able to start a run.
pub fn head(workspace: &Path) -> Option<String> {
    use crate::git::Git;
    crate::git::GitCli.head(workspace)
}

/// `git rev-parse --abbrev-ref HEAD`, the source of `RunState::branch`.
/// `None` on any failure; a detached HEAD resolves to `"HEAD"`.
pub fn branch(workspace: &Path) -> Option<String> {
    use crate::git::Git;
    crate::git::GitCli.branch(workspace)
}

/// Every path this workspace has changed relative to `base`, combining
/// `git diff --name-only <base>` (committed changes) with
/// `git status --porcelain --untracked-files=all` (uncommitted changes, an untracked folder listed file by file, still sitting in the working
/// tree when `reconcile` runs). The second source already honors
/// `.gitignore`, so build artifacts never show up as noise. A rename
/// (`git status`'s `R  old -> new`) reports only the new path — the one that
/// actually exists for a scope check to classify. Deduplicated and sorted
/// (`BTreeSet`); empty (never an error) when either command fails, since a
/// caller cannot verify scope over a diff it could not compute anyway.
pub fn changed(workspace: &Path, base: &str) -> Vec<String> {
    use crate::git::Git;
    crate::git::GitCli.changed(workspace, base)
}

/// The body of [`changed`], parameterized over the `git` capture function
/// so [`crate::git::GitCli`] owns the spawn and this module the parsing.
pub(crate) fn changed_with(
    workspace: &Path,
    base: &str,
    capture: fn(&Path, &[&str]) -> Option<String>,
) -> Vec<String> {
    let mut set = BTreeSet::new();

    // LOC-27: `base` reaches git as an argument; refuse anything that is not a
    // plain commit id so it can never be parsed as an option (`--output=x`).
    if let Err(e) = validate_base_sha(base) {
        tracing::warn!(error = %e, "rejected base sha");
        return Vec::new();
    }

    if let Some(diff) = capture(workspace, &["diff", "--name-only", base, "--"]) {
        for line in diff.lines() {
            let p = line.trim();
            if !p.is_empty() {
                set.insert(normalize_path(p));
            }
        }
    }

    if let Some(status) = capture(
        workspace,
        &["status", "--porcelain", "--untracked-files=all"],
    ) {
        for line in status.lines() {
            // Porcelain v1: two status chars, one space, then the path (or,
            // for a rename/copy, `old -> new`).
            if line.len() < 4 {
                continue;
            }
            let rest = &line[3..];
            let path = match rest.find(" -> ") {
                Some(idx) => &rest[idx + 4..],
                None => rest,
            };
            let p = path.trim().trim_matches('"');
            if !p.is_empty() {
                set.insert(normalize_path(p));
            }
        }
    }

    set.into_iter().collect()
}

/// LOC-27: a base revision must be 7-64 ASCII hex characters (an abbreviated
/// or full SHA-1/SHA-256 commit id). Anything else is refused, naming it.
pub(crate) fn validate_base_sha(base: &str) -> Result<(), String> {
    let ok = (7..=64).contains(&base.len()) && base.bytes().all(|b| b.is_ascii_hexdigit());
    if ok {
        Ok(())
    } else {
        Err(format!(
            "invalid base revision {base:?}: expected 7-64 hex characters"
        ))
    }
}

fn normalize_path(p: &str) -> String {
    p.replace('\\', "/")
}

fn fix_keyword_regex() -> &'static regex::Regex {
    static RE: once_cell::sync::OnceCell<regex::Regex> = once_cell::sync::OnceCell::new();
    RE.get_or_init(|| regex::Regex::new(r"(?i)\b(fix|hotfix|patch|revert|bugfix|fixup)\b").unwrap())
}

/// PURE: for every commit, its later fate within `window` subsequent commits
/// (by history order, i.e. commits that come *before* it in `commits` — `git
/// log` lists newest first, so index `j < i` is chronologically later than
/// index `i`).
///
/// Priority per commit, checked in order (mirrors the spec: revert beats
/// hotfix beats success — a commit that is both exactly reverted and later
/// patched is reported as reverted, the more decisive outcome):
///   1. [`Verdict::Reverted`] — some later commit's subject is exactly
///      `Revert "<this commit's subject>"`.
///   2. [`Verdict::Hotfixed`] — some later commit's subject matches
///      `\b(fix|hotfix|patch|revert|bugfix|fixup)\b` (case-insensitive) AND
///      shares at least `overlap` of its files, the fraction computed against
///      `min(|files_i|, |files_j|)` — the **smaller** of the two file sets —
///      so a small, tightly-targeted fix on top of a large multi-file change
///      still registers as a hotfix instead of being diluted by the big
///      commit's unrelated files.
///   3. [`Verdict::Success`] otherwise.
///
/// A commit with an empty file set can never be hotfixed (the overlap
/// denominator would be zero) — it can still be exactly reverted.
pub fn verdicts(commits: &[Commit], window: usize, overlap: f32) -> BTreeMap<String, Verdict> {
    let mut out = BTreeMap::new();
    let fix_re = fix_keyword_regex();

    for i in 0..commits.len() {
        let c = &commits[i];
        let start = i.saturating_sub(window);
        let mut verdict = Verdict::Success;

        // 1. Exact revert, nearest match first.
        let revert_subject = format!("Revert \"{}\"", c.subject);
        for later in commits[start..i].iter().rev() {
            if later.subject.trim() == revert_subject {
                verdict = Verdict::Reverted {
                    by: later.sha.clone(),
                };
                break;
            }
        }

        // 2. Hotfix, only when not already reverted.
        if matches!(verdict, Verdict::Success) {
            for later in commits[start..i].iter().rev() {
                if !fix_re.is_match(&later.subject) {
                    continue;
                }
                let denom = c.files.len().min(later.files.len());
                if denom == 0 {
                    continue;
                }
                let inter = c.files.intersection(&later.files).count();
                let ratio = inter as f32 / denom as f32;
                if ratio >= overlap {
                    verdict = Verdict::Hotfixed {
                        by: later.sha.clone(),
                        overlap: ratio,
                    };
                    break;
                }
            }
        }

        out.insert(c.sha.clone(), verdict);
    }

    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn validate_base_sha_refuses_non_hex() {
        for bad in ["--output=x", "HEAD", ""] {
            let err = super::validate_base_sha(bad).unwrap_err();
            assert!(err.contains(&format!("{bad:?}")), "{err}");
        }
        assert!(super::validate_base_sha("abcdef1").is_ok());
        assert!(super::validate_base_sha(&"a1".repeat(20)).is_ok());
    }

    #[test]
    fn changed_refuses_option_like_base() {
        assert!(super::changed(std::path::Path::new("."), "--output=x").is_empty());
    }

    /// A wholly untracked folder is listed file by file, as the committed diff lists it once the files are committed.
    #[test]
    fn changed_lists_the_files_of_an_untracked_folder() {
        fn capture(_: &std::path::Path, args: &[&str]) -> Option<String> {
            match args.first() {
                Some(&"status") if args.contains(&"--untracked-files=all") => Some(
                    "?? feature/a.cs
?? feature/b.cs"
                        .to_string(),
                ),
                Some(&"status") => Some("?? feature/".to_string()),
                _ => None,
            }
        }
        let changed = super::changed_with(std::path::Path::new("."), "abcdef1", capture);
        assert_eq!(
            changed,
            vec!["feature/a.cs".to_string(), "feature/b.cs".to_string()]
        );
    }

    use super::*;
    use aidp_testkit::TempWorkspace;
    use std::process::Command;

    fn fixture_raw() -> String {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/gitlog/log-name-only.txt"
        );
        std::fs::read_to_string(path).expect("gitlog fixture must exist")
    }

    fn tmp_repo(label: &str) -> TempWorkspace {
        let ws = TempWorkspace::new(label);
        let dir = ws.path();
        let run = |args: &[&str]| {
            let st = Command::new("git")
                .args(args)
                .current_dir(dir)
                .status()
                .expect("git must be on PATH for this test");
            assert!(st.success(), "git {:?} failed", args);
        };
        run(&["init", "-q"]);
        run(&["config", "user.email", "test@example.com"]);
        run(&["config", "user.name", "aidp tests"]);
        ws
    }

    // --- pure: parse_log -----------------------------------------------

    #[test]
    fn parse_log_handles_multiple_separated_commits() {
        let raw = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\u{1f}feat: add thing\u{1f}2026-01-01T00:00:00Z\nsrc/a.rs\nsrc/b.rs\n\nbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\u{1f}fix: fix thing\u{1f}2026-01-02T00:00:00Z\nsrc/a.rs\n";
        let commits = parse_log(raw);
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].sha, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
        assert_eq!(commits[0].subject, "feat: add thing");
        assert_eq!(commits[0].files.len(), 2);
        assert!(commits[0].files.contains("src/a.rs"));
        assert_eq!(commits[1].files.len(), 1);
    }

    #[test]
    fn parse_log_handles_a_commit_with_no_files() {
        let raw =
            "cccccccccccccccccccccccccccccccccccccccc\u{1f}chore: empty\u{1f}2026-01-03T00:00:00Z\n";
        let commits = parse_log(raw);
        assert_eq!(commits.len(), 1);
        assert!(commits[0].files.is_empty());
    }

    #[test]
    fn parse_log_on_empty_input_is_empty() {
        assert!(parse_log("").is_empty());
        assert!(parse_log("\n\n\n").is_empty());
    }

    // --- pure: verdicts, driven by the committed fixture ----------------
    //
    // The fixture encodes four scenarios (see tests/fixtures/gitlog/README
    // section in the file header comment): an exact revert, a hotfix at 60%
    // overlap (triggers), one at 30% (does not, threshold 0.5), and a fix
    // commit more than 20 commits later than its target (out of window, does
    // not trigger regardless of overlap).

    #[test]
    fn it_verdicts_detects_an_exact_revert() {
        let commits = parse_log(&fixture_raw());
        let v = verdicts(&commits, 20, 0.5);
        let target = commits
            .iter()
            .find(|c| c.subject == "feat: add experimental cache layer")
            .expect("revert target present in fixture");
        match &v[&target.sha] {
            Verdict::Reverted { by } => {
                let reverter = commits.iter().find(|c| &c.sha == by).unwrap();
                assert_eq!(
                    reverter.subject,
                    "Revert \"feat: add experimental cache layer\""
                );
            }
            other => panic!("expected Reverted, got {other:?}"),
        }
    }

    #[test]
    fn it_verdicts_detects_a_hotfix_at_60_percent_overlap() {
        let commits = parse_log(&fixture_raw());
        let v = verdicts(&commits, 20, 0.5);
        let target = commits
            .iter()
            .find(|c| c.subject == "feat: add rate limiter for auth service")
            .expect("60% hotfix target present in fixture");
        match &v[&target.sha] {
            Verdict::Hotfixed { overlap, .. } => {
                assert!(
                    (*overlap - 0.6).abs() < 0.01,
                    "expected ~0.6 overlap, got {overlap}"
                );
            }
            other => panic!("expected Hotfixed, got {other:?}"),
        }
    }

    #[test]
    fn it_verdicts_does_not_trigger_a_hotfix_at_30_percent_overlap() {
        let commits = parse_log(&fixture_raw());
        let v = verdicts(&commits, 20, 0.5);
        let target = commits
            .iter()
            .find(|c| c.subject == "feat: add pagination to reports API")
            .expect("30% hotfix target present in fixture");
        assert!(
            matches!(v[&target.sha], Verdict::Success),
            "a 30% overlap must not clear a 0.5 threshold: {:?}",
            v[&target.sha]
        );
    }

    #[test]
    fn it_verdicts_ignores_a_fix_outside_the_window() {
        let commits = parse_log(&fixture_raw());
        let v = verdicts(&commits, 20, 0.5);
        let target = commits
            .iter()
            .find(|c| c.subject == "feat: add outcome scoring for D scenario")
            .expect("out-of-window target present in fixture");
        assert!(
            matches!(v[&target.sha], Verdict::Success),
            "a fix more than `window` commits later must not count: {:?}",
            v[&target.sha]
        );
    }

    // --- impure: against a real (throwaway) repository ------------------

    #[test]
    fn it_head_and_branch_and_log_round_trip_on_a_real_repo() {
        let ws = tmp_repo("roundtrip");
        let dir = ws.path();
        std::fs::write(dir.join("a.txt"), "hello").unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(dir)
            .status()
            .unwrap();
        Command::new("git")
            .args(["commit", "-q", "-m", "feat: first commit"])
            .current_dir(dir)
            .status()
            .unwrap();

        let sha = head(dir).expect("HEAD must resolve on a repo with a commit");
        assert_eq!(sha.len(), 40);

        let b = branch(dir).expect("branch must resolve");
        assert!(!b.is_empty());

        let commits = log(dir, 10, None).expect("log must succeed on a real repo");
        assert_eq!(commits.len(), 1);
        assert_eq!(commits[0].sha, sha);
        assert_eq!(commits[0].subject, "feat: first commit");
        assert!(commits[0].files.contains("a.txt"));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn head_is_none_outside_a_repository() {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("aidp-gitlog-norepo-{nanos}"));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(head(&dir).is_none());
        assert!(changed(&dir, "HEAD").is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn it_changed_combines_committed_diff_and_working_tree_status() {
        let ws = tmp_repo("changed");
        let dir = ws.path();
        std::fs::write(dir.join("base.txt"), "base").unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(dir)
            .status()
            .unwrap();
        Command::new("git")
            .args(["commit", "-q", "-m", "feat: base"])
            .current_dir(dir)
            .status()
            .unwrap();
        let base = head(dir).unwrap();

        // A committed follow-up change...
        std::fs::write(dir.join("committed.txt"), "x").unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(dir)
            .status()
            .unwrap();
        Command::new("git")
            .args(["commit", "-q", "-m", "feat: committed change"])
            .current_dir(dir)
            .status()
            .unwrap();

        // ...plus an uncommitted one sitting in the working tree.
        std::fs::write(dir.join("dirty.txt"), "y").unwrap();

        let changed_files = changed(dir, &base);
        assert!(changed_files.contains(&"committed.txt".to_string()));
        assert!(changed_files.contains(&"dirty.txt".to_string()));

        let _ = std::fs::remove_dir_all(dir);
    }

    /// Regression test for a real bug found while merging this lot: when the
    /// alphabetically-first path in `git status --porcelain`'s output is a
    /// TRACKED file with an unstaged-only modification, its status line
    /// starts with a literal space (`" M AGENTS.md"`), and that space is also
    /// the very first byte of the whole captured stdout. `run_git` used to
    /// call the blanket `.trim()`, which ate that leading space, shifting
    /// `changed`'s `&line[3..]` slice one character INTO the filename and
    /// producing a corrupted extra entry (`"AGENTS.md"` -> `"GENTS.md"`)
    /// alongside the correct one, rather than replacing it — silent
    /// corruption, not a loud failure. An untracked (`??`) or staged (`M `)
    /// first entry never triggers it, which is why the pre-existing test
    /// above (whose first status line is `"?? dirty.txt"`) did not catch it.
    #[test]
    fn it_changed_does_not_corrupt_the_alphabetically_first_status_line() {
        let ws = tmp_repo("no-corrupt");
        let dir = ws.path();
        // "aaa.txt" sorts before "zzz.txt", so it is the FIRST line `git
        // status --porcelain` prints — the one a blanket `.trim()` would eat
        // the leading status space off of.
        std::fs::write(dir.join("aaa.txt"), "one").unwrap();
        std::fs::write(dir.join("zzz.txt"), "two").unwrap();
        Command::new("git")
            .args(["add", "."])
            .current_dir(dir)
            .status()
            .unwrap();
        Command::new("git")
            .args(["commit", "-q", "-m", "feat: base"])
            .current_dir(dir)
            .status()
            .unwrap();
        let base = head(dir).unwrap();

        // Both tracked files modified, unstaged only: both status lines are
        // `" M <path>"`, the first one starting with the vulnerable leading
        // space.
        std::fs::write(dir.join("aaa.txt"), "one-changed").unwrap();
        std::fs::write(dir.join("zzz.txt"), "two-changed").unwrap();

        let changed_files = changed(dir, &base);
        assert!(
            changed_files.contains(&"aaa.txt".to_string()),
            "the alphabetically-first changed file must survive whole: {changed_files:?}"
        );
        assert!(changed_files.contains(&"zzz.txt".to_string()));
        assert!(
            !changed_files.iter().any(|p| p.len() < "aaa.txt".len()),
            "no entry should ever be shorter than the shortest real path — a \
             truncated entry means the leading status column leaked into the \
             filename: {changed_files:?}"
        );

        let _ = std::fs::remove_dir_all(dir);
    }
}
