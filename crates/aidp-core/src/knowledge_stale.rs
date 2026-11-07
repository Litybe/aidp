//! Chantier C3 du Lot 3 — dating a feature note's `updated:` claim against
//! real git history, via the `files:` key added to [`crate::knowledge::FeatureNote`].
//!
//! `updated: 2026-01-10` on a note whose `files:` were refactored in March is
//! indistinguishable from an accurate note — until something actually reads
//! git. This module is that something, split deliberately in two:
//!
//! * [`classify`] is **pure**: given dates already collected (a note, paired
//!   with each of its `files:` and that file's last-commit timestamp, or
//!   `None` when the file is gone or has no history), it decides which notes
//!   are behind. No I/O, fully testable on a hand-built fixture.
//! * [`stale`] is **impure**: it walks the knowledge base, shells out to `git
//!   log` per file, and feeds the result to `classify`.
//!
//! Contract: **non-blocking**, mirroring [`crate::kg::ensure`]'s header — git
//! missing, the workspace not a git repository, or a note with no `files:` at
//! all produces no signal for that note, never an error, never a non-zero
//! process exit, never a panic.
//!
//! Cost, and why this never runs on the prompt-building hot path: ~80 notes ×
//! ~6 files each is ~500 process spawns — fine for a standalone `aidp
//! knowledge stale` command, not something `crate::prompt` can afford while
//! rendering a Task prompt. So `stale`'s result is cached at
//! `<knowledge_dir>/.aidp-stale.json` (see [`cache_path`]), and `crate::prompt`
//! only ever reads that cache — it is already excluded from
//! [`crate::knowledge::list`] and [`crate::knowledge::root_notes`] (both skip
//! dotfiles), so no extra filter is needed there.

use crate::git::{Git, GitCli};
use crate::knowledge::FeatureNote;
use std::path::{Path, PathBuf};

/// One note's staleness reading. Present in [`classify`]'s output only for a
/// note that declares at least one `files:` entry — a note without the key
/// carries no signal, by design (never a false positive).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StaleNote {
    pub slug: String,
    pub path: PathBuf,
    pub updated: Option<String>,
    /// RFC3339 timestamp of the most recent commit touching any of the note's
    /// `files:`, across the ones that still have git history.
    pub newest_change: Option<String>,
    /// Days between `newest_change` and `updated` (0 when either is missing
    /// or unparseable, or when the note is not behind).
    pub days_behind: i64,
    /// `files:` entries that no longer exist on disk.
    pub missing_files: Vec<String>,
}

/// The filename of the cache `stale` writes and `crate::prompt` reads, under
/// the knowledge base root (a dotfile, so [`crate::knowledge::list`] and
/// [`crate::knowledge::root_notes`] already skip it — see their `!n.starts_with('.')` filters).
pub const STALE_CACHE_FILE: &str = ".aidp-stale.json";

/// `<knowledge_dir>/.aidp-stale.json`.
pub fn cache_path(knowledge_dir: &Path) -> PathBuf {
    knowledge_dir.join(STALE_CACHE_FILE)
}

/// Best-effort read of a previously written [`stale`] result. `crate::prompt`
/// calls only this — never `stale` itself — so building a Task prompt never
/// shells out to git. Missing file, unreadable, or unparseable all degrade to
/// an empty list rather than an error: the peremption warning is a nice-to-have,
/// never a reason a prompt fails to render.
pub fn load_cache(knowledge_dir: &Path) -> Vec<StaleNote> {
    let Ok(text) = std::fs::read_to_string(cache_path(knowledge_dir)) else {
        return Vec::new();
    };
    serde_json::from_str(&text).unwrap_or_default()
}

/// A feature note paired with, for every path in its `files:`, that path and
/// its last commit timestamp (`None` = file gone, untracked, or no history).
/// Purely a readability alias for [`classify`]'s input — structurally the
/// exact tuple the spec's original skeleton signature named.
pub type NoteFiles = (FeatureNote, Vec<(String, Option<String>)>);

/// PURE: decide staleness from dates already collected. `notes` pairs each
/// feature note with, for every path in its `files:`, that path and its last
/// commit timestamp (`None` = file gone, untracked, or no history).
///
/// A note whose `files:` is empty is skipped entirely — nothing to compare
/// against, so reporting it would be a guess dressed up as a finding, exactly
/// the kind of false positive a staleness check must never produce.
pub fn classify(notes: &[NoteFiles]) -> Vec<StaleNote> {
    let mut out = Vec::new();
    for (note, files) in notes {
        if files.is_empty() {
            continue;
        }

        let missing_files: Vec<String> = files
            .iter()
            .filter(|(_, ts)| ts.is_none())
            .map(|(f, _)| f.clone())
            .collect();

        let mut newest_change: Option<String> = None;
        let mut newest_dt: Option<chrono::DateTime<chrono::FixedOffset>> = None;
        for (_, ts) in files {
            let Some(ts) = ts else { continue };
            let Ok(dt) = chrono::DateTime::parse_from_rfc3339(ts) else {
                continue;
            };
            if newest_dt.map(|n| dt > n).unwrap_or(true) {
                newest_dt = Some(dt);
                newest_change = Some(ts.clone());
            }
        }

        let days_behind = match (newest_dt, note.updated.as_deref().and_then(parse_note_date)) {
            (Some(newest), Some(updated)) => (newest.date_naive() - updated).num_days().max(0),
            _ => 0,
        };

        out.push(StaleNote {
            slug: note.slug.clone(),
            path: note.path.clone(),
            updated: note.updated.clone(),
            newest_change,
            days_behind,
            missing_files,
        });
    }
    out
}

/// A note's `updated:` value, as either a full RFC3339 timestamp or a bare
/// `YYYY-MM-DD` date (the form every template's frontmatter contract asks
/// for) — either way, reduced to a calendar date for the day-granularity
/// comparison [`classify`] needs.
fn parse_note_date(s: &str) -> Option<chrono::NaiveDate> {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return Some(dt.date_naive());
    }
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()
}

/// IMPURE: walks every feature note's `files:`, checking each one's presence
/// on disk and (if present) its last commit date via `git log`, then hands
/// the result to [`classify`].
///
/// Non-blocking at two levels: if `workspace_root` is not usable as a git
/// repository at all (git missing, or the directory is not a repo), this
/// returns an empty `Vec` immediately — no per-file git calls are attempted,
/// so a workspace without git never gets a false "missing" signal just
/// because nothing could be checked. Within a usable repo, a per-file
/// `path.exists()` check (not the absence of a git timestamp) is what marks a
/// file as gone — a file that exists but happens to be untracked still gets a
/// timestamp attempt, and simply contributes no `newest_change` data if git
/// has nothing to say about it.
pub fn stale(knowledge_dir: &Path, workspace_root: &Path) -> Vec<StaleNote> {
    stale_with(knowledge_dir, workspace_root, &GitCli)
}

/// Same contract as [`stale`], through an injected [`Git`] port so a test can
/// supply canned commit dates instead of shelling out.
pub fn stale_with(knowledge_dir: &Path, workspace_root: &Path, git: &dyn Git) -> Vec<StaleNote> {
    if !git.is_usable(workspace_root) {
        return Vec::new();
    }
    let notes = crate::knowledge::list(knowledge_dir);
    let collected: Vec<NoteFiles> = notes
        .into_iter()
        .map(|n| {
            let files: Vec<(String, Option<String>)> = n
                .files
                .iter()
                .map(|f| {
                    let exists = workspace_root.join(f).exists();
                    let ts = if exists {
                        git.last_commit_date(workspace_root, f)
                    } else {
                        None
                    };
                    (f.clone(), ts)
                })
                .collect();
            (n, files)
        })
        .collect();
    classify(&collected)
}

#[cfg(test)]
mod tests {
    use super::*;
    use aidp_testkit::TempWorkspace;

    fn note(slug: &str, updated: Option<&str>) -> FeatureNote {
        FeatureNote {
            path: PathBuf::from(format!("{slug}.md")),
            slug: slug.to_string(),
            name: slug.replace('-', " "),
            aliases: Vec::new(),
            status: "active".into(),
            tickets: Vec::new(),
            updated: updated.map(|s| s.to_string()),
            area: None,
            files: Vec::new(),
            valid_until: None,
            superseded_by: None,
            findings: Vec::new(),
            bytes: 0,
            guard_off: false,
        }
    }

    #[test]
    fn classify_flags_the_late_note_ignores_the_fresh_one_and_skips_notes_without_files() {
        let mut up_to_date = note("up-to-date", Some("2026-03-01"));
        up_to_date.files = vec!["src/a.rs".to_string()];

        let mut late = note("late-by-120-days", Some("2026-01-01"));
        late.files = vec!["src/b.rs".to_string()];

        let mut files_gone = note("files-gone", Some("2026-01-01"));
        files_gone.files = vec!["src/c.rs".to_string(), "src/d.rs".to_string()];

        // No `files:` at all — must never appear in the result, even though
        // its `updated:` is just as old as `files_gone`'s.
        let never_a_false_positive = note("no-files-declared", Some("2020-01-01"));

        let input = vec![
            (
                up_to_date.clone(),
                vec![(
                    "src/a.rs".to_string(),
                    Some("2026-03-01T00:00:00Z".to_string()),
                )],
            ),
            (
                late.clone(),
                vec![(
                    "src/b.rs".to_string(),
                    Some("2026-05-01T00:00:00Z".to_string()),
                )],
            ),
            (
                files_gone.clone(),
                vec![
                    ("src/c.rs".to_string(), None),
                    ("src/d.rs".to_string(), None),
                ],
            ),
            (never_a_false_positive, Vec::new()),
        ];

        let out = classify(&input);
        assert_eq!(
            out.len(),
            3,
            "the note without files: must be absent, never a false positive: {out:?}"
        );
        assert!(
            !out.iter().any(|s| s.slug == "no-files-declared"),
            "{out:?}"
        );

        let ud = out.iter().find(|s| s.slug == "up-to-date").unwrap();
        assert_eq!(ud.days_behind, 0);
        assert!(ud.missing_files.is_empty());

        let lt = out.iter().find(|s| s.slug == "late-by-120-days").unwrap();
        assert_eq!(lt.days_behind, 120, "Jan 1 -> May 1 is 120 days");
        assert!(lt.missing_files.is_empty());

        let fg = out.iter().find(|s| s.slug == "files-gone").unwrap();
        assert_eq!(fg.missing_files, vec!["src/c.rs", "src/d.rs"]);
    }

    #[test]
    fn stale_on_a_directory_without_git_returns_empty_and_does_not_error() {
        // This IS the non-blocking contract: a knowledge base whose workspace
        // is not a git repository (or has no git at all) must produce no
        // signal, not a false "everything is missing", and must never panic
        // or return an error type.
        let temp_ws = TempWorkspace::new("stale-no-git");
        let ws = temp_ws.path().to_path_buf();
        let knowledge_dir = ws.join("Knowledge");
        let features = crate::knowledge::features_dir(&knowledge_dir);
        std::fs::create_dir_all(&features).unwrap();
        // A real file, actually present on disk, so a false "missing" would
        // be caused only by mishandling the git-absent case.
        std::fs::write(ws.join("real.rs"), "fn main() {}").unwrap();
        std::fs::write(
            features.join("some-feature.md"),
            "---\nfeature: Some Feature\nslug: some-feature\nupdated: 2020-01-01\nfiles: [real.rs]\n---\n\n# Some Feature\n",
        )
        .unwrap();

        let out = stale(&knowledge_dir, &ws);
        assert!(
            out.is_empty(),
            "a non-git workspace must yield no signal at all: {out:?}"
        );

        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn load_cache_degrades_to_empty_without_touching_git() {
        let temp_ws = TempWorkspace::new("stale-cache");
        let dir = temp_ws.path().to_path_buf();

        // No cache file yet.
        assert!(load_cache(&dir).is_empty());

        let payload = vec![StaleNote {
            slug: "x".into(),
            path: dir.join("Features/x.md"),
            updated: Some("2026-01-01".into()),
            newest_change: Some("2026-06-01T00:00:00Z".into()),
            days_behind: 151,
            missing_files: Vec::new(),
        }];
        std::fs::write(cache_path(&dir), serde_json::to_string(&payload).unwrap()).unwrap();
        let loaded = load_cache(&dir);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].slug, "x");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
