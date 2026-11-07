//! Chantier C4 du Lot 3 — `aidp knowledge gc`: a read-only report on the
//! knowledge base's structural rot, never a mutation.
//!
//! Nothing in `aidp-core` deletes, archives or merges a feature note:
//! [`crate::obsidian`] only writes, [`crate::knowledge`] only reads. Left
//! alone, the base can only grow — and the module docs of [`crate::knowledge`]
//! name its characteristic failure mode: a **fork**
//! (`auto-exclusion.md` / `autoexclusion.md` / `auto-exclusion-v2.md`). Once a
//! fork exists — a weak `create` on a bare ticket key, a hand edit, a badly
//! resolved `ambiguous` — nothing detects it: both notes keep resolving as
//! candidates, and each run updates whichever one it happens to land on.
//!
//! This module is the detector, and only the detector: [`report`] reads the
//! base and returns three lists. It never writes, moves or deletes a note —
//! the base is a vault the user owns and edits by hand, with no trash can to
//! recover from a wrong guess. The only actionable output is a command line
//! to copy: consolidating two notes into one is prose work for a human (or an
//! agent told to do exactly that), never a byte-level operation this module
//! could perform safely.
//!
//! Explicitly out of scope, and not to be added back: a `sweep_superseded`
//! that stamps `status: archived`, a `--move-archived` that relocates notes
//! under `Features/_archive/`, and a `compact`. Moving a note breaks the
//! absolute wikilinks [`crate::obsidian::link_or_text`] already wrote
//! elsewhere in the vault; `Features/_archive/` would be invisible to
//! [`crate::knowledge::list`] (which skips the `_` prefix — see
//! [`crate::vault::NOTE_PREFIX`]), silently erasing from the index exactly the
//! history `Decision::Supersede` exists to keep visible; and
//! [`crate::obsidian::refresh_knowledge`] already covers re-indexing via
//! `aidp knowledge index`.

use crate::knowledge::FeatureNote;
use std::collections::BTreeSet;
use std::path::Path;

/// Two or more notes whose identity looks like the same business feature.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ForkGroup {
    pub slugs: Vec<String>,
    pub reason: String,
    /// A ready-to-paste `aidp knowledge supersede` line — never run
    /// automatically. The suggestion always points the OLDER-looking note
    /// (`--slug`) at the more recently updated one (`--by`); a human still
    /// has to decide these are really the same feature.
    pub suggested_command: String,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct GcReport {
    pub forks: Vec<ForkGroup>,
    /// `"<slug> -> <dead superseded_by target>"`, one per note whose pointer
    /// resolves to nothing in the base.
    pub dangling: Vec<String>,
    /// Slugs of notes whose `files:` is non-empty and every entry is gone
    /// from `workspace_root` — candidates to `supersede`, never to delete.
    pub all_files_gone: Vec<String>,
}

/// Jaccard similarity threshold above which two notes' handle vocabularies
/// are close enough to flag as a likely fork, even with no exact handle match.
const FORK_JACCARD_THRESHOLD: f32 = 0.8;

/// Read-only: three passes over `crate::knowledge::list`, no writes, no
/// moves, no deletes — verified by `report_never_writes_to_disk` below.
pub fn report(knowledge_dir: &Path, workspace_root: &Path) -> GcReport {
    let notes = crate::knowledge::list(knowledge_dir);
    GcReport {
        forks: detect_forks(&notes),
        dangling: detect_dangling(&notes),
        all_files_gone: detect_all_files_gone(&notes, workspace_root),
    }
}

/// Cartesian product over pairs of notes (never a note against itself, never
/// a pair twice): flagged when normalized handles collide exactly, OR the
/// Jaccard of their handle tokens is >= [`FORK_JACCARD_THRESHOLD`], OR they
/// share a `tickets:` entry. Deliberately no content hashing: two prose notes
/// about the very same feature essentially never hash equal, which is exactly
/// the fork this detector exists to catch — a hash check would miss it every
/// time.
fn detect_forks(notes: &[FeatureNote]) -> Vec<ForkGroup> {
    // Precomputed once per note (O(n)) instead of recomputing the normalized
    // handles and the handle-token set for every pair the O(n²) loop below
    // visits.
    let precomputed: Vec<(Vec<String>, BTreeSet<String>)> = notes
        .iter()
        .map(|n| {
            let handles = n
                .handles()
                .iter()
                .map(|h| crate::knowledge::normalize(h))
                .collect();
            (handles, handle_token_set(n))
        })
        .collect();

    let mut groups = Vec::new();
    for i in 0..notes.len() {
        for j in (i + 1)..notes.len() {
            let a = &notes[i];
            let b = &notes[j];
            let mut reasons: Vec<String> = Vec::new();

            let (a_handles, ta) = &precomputed[i];
            let (b_handles, tb) = &precomputed[j];
            if a_handles
                .iter()
                .any(|na| !na.is_empty() && b_handles.contains(na))
            {
                reasons.push("identical handle once normalized".to_string());
            }

            let jac = jaccard(ta, tb);
            if jac >= FORK_JACCARD_THRESHOLD {
                reasons.push(format!(
                    "{:.0}% handle-token overlap (Jaccard)",
                    jac * 100.0
                ));
            }

            let shared_tickets: Vec<&str> = a
                .tickets
                .iter()
                .filter(|t| b.tickets.iter().any(|u| u.eq_ignore_ascii_case(t)))
                .map(|s| s.as_str())
                .collect();
            if !shared_tickets.is_empty() {
                reasons.push(format!("shares ticket(s) {}", shared_tickets.join(", ")));
            }

            if reasons.is_empty() {
                continue;
            }

            let (winner, loser) = pick_winner_loser(a, b);
            let mut slugs = vec![a.slug.clone(), b.slug.clone()];
            slugs.sort();
            groups.push(ForkGroup {
                slugs,
                reason: reasons.join("; "),
                suggested_command: format!(
                    "aidp knowledge supersede --slug {} --by {}",
                    loser.slug, winner.slug
                ),
            });
        }
    }
    groups
}

/// The union of tokens across every handle of a note — a cheap stand-in for
/// "what this note is called", used only for the Jaccard comparison.
fn handle_token_set(n: &FeatureNote) -> BTreeSet<String> {
    let mut set = BTreeSet::new();
    for h in n.handles() {
        set.extend(crate::knowledge::tokens(&crate::knowledge::normalize(&h)));
    }
    set
}

fn jaccard(a: &BTreeSet<String>, b: &BTreeSet<String>) -> f32 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let inter = a.intersection(b).count() as f32;
    let union = a.union(b).count() as f32;
    if union == 0.0 {
        0.0
    } else {
        inter / union
    }
}

/// Which of a fork pair to keep (`--by`) vs. suggest superseding (`--slug`):
/// the more recently `updated:` note wins; ties (or missing dates on either
/// side) fall back to slug order so the suggestion is at least deterministic
/// — a human reviews it either way.
fn pick_winner_loser<'a>(
    a: &'a FeatureNote,
    b: &'a FeatureNote,
) -> (&'a FeatureNote, &'a FeatureNote) {
    match (&a.updated, &b.updated) {
        (Some(ua), Some(ub)) if ua != ub => {
            if ua > ub {
                (a, b)
            } else {
                (b, a)
            }
        }
        _ if a.slug <= b.slug => (b, a),
        _ => (a, b),
    }
}

/// Every `superseded_by` that names a slug absent from the base — a dead link
/// is an authoring mistake, and `resolve` already refuses to treat it as a
/// green light for `create` (see `Decision::Ambiguous`'s broken-pointer
/// branch); `gc` is what makes the mistake visible without waiting for a run
/// to stumble into it.
fn detect_dangling(notes: &[FeatureNote]) -> Vec<String> {
    let mut out: Vec<String> = notes
        .iter()
        .filter_map(|n| {
            let by = n.superseded_by.as_ref()?;
            let by_slug = crate::knowledge::slugify(by);
            let exists = notes.iter().any(|m| m.slug == by_slug);
            (!exists).then(|| format!("{} -> {}", n.slug, by))
        })
        .collect();
    out.sort();
    out
}

/// Notes whose `files:` is non-empty and every listed path is gone from
/// `workspace_root`. A note with an empty `files:` never appears here — it
/// declared nothing to check, so "all of nothing is gone" would be a
/// meaningless true.
fn detect_all_files_gone(notes: &[FeatureNote], workspace_root: &Path) -> Vec<String> {
    let mut out: Vec<String> = notes
        .iter()
        .filter(|n| !n.files.is_empty())
        .filter(|n| n.files.iter().all(|f| !workspace_root.join(f).exists()))
        .map(|n| n.slug.clone())
        .collect();
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ops::Deref;

    /// A temporary fixture directory that owns a TempWorkspace guard.
    /// This wrapper ensures the directory is cleaned up even on panic.
    struct FixtureDir {
        workspace: aidp_testkit::TempWorkspace,
    }

    impl Deref for FixtureDir {
        type Target = Path;

        fn deref(&self) -> &Path {
            self.workspace.path()
        }
    }

    fn fixture(name: &str) -> FixtureDir {
        let workspace = aidp_testkit::TempWorkspace::new(&format!("knowledge-gc-{name}"));
        std::fs::create_dir_all(crate::knowledge::features_dir(workspace.path())).unwrap();
        FixtureDir { workspace }
    }

    fn write_feature(dir: &Path, file: &str, body: &str) {
        std::fs::write(crate::knowledge::features_dir(dir).join(file), body).unwrap();
    }

    #[test]
    fn two_notes_sharing_a_name_fork_the_third_unrelated_note_does_not() {
        let dir = fixture("forks");
        write_feature(
            &dir,
            "auto-exclusion.md",
            "---\nfeature: Auto Exclusion Joueur\nslug: auto-exclusion\nupdated: 2026-01-01\n---\n\n# Auto Exclusion Joueur\n",
        );
        write_feature(
            &dir,
            "autoexclusion.md",
            "---\nfeature: Auto Exclusion Joueur\nslug: autoexclusion\nupdated: 2026-03-01\n---\n\n# Auto Exclusion Joueur (bis)\n",
        );
        write_feature(
            &dir,
            "limites-de-depot.md",
            "---\nfeature: Limites de depot\nslug: limites-de-depot\n---\n\n# Limites de depot\n",
        );

        let r = report(&dir, &dir);
        assert_eq!(r.forks.len(), 1, "{:?}", r.forks);
        let group = &r.forks[0];
        assert_eq!(group.slugs, vec!["auto-exclusion", "autoexclusion"]);
        assert!(
            !group.slugs.contains(&"limites-de-depot".to_string()),
            "the unrelated third note must never join the fork group"
        );
        // The more recently updated note (autoexclusion, 2026-03-01) is
        // suggested as the survivor.
        assert_eq!(
            group.suggested_command,
            "aidp knowledge supersede --slug auto-exclusion --by autoexclusion"
        );
        assert!(r.dangling.is_empty());
        assert!(r.all_files_gone.is_empty());
    }

    #[test]
    fn a_dead_superseded_by_pointer_is_reported_as_dangling() {
        let dir = fixture("dangling");
        write_feature(
            &dir,
            "old-note.md",
            "---\nfeature: Old Note\nslug: old-note\nstatus: deprecated\nsuperseded_by: inexistant\n---\n\n# Old Note\n",
        );

        let r = report(&dir, &dir);
        assert_eq!(r.dangling.len(), 1, "{:?}", r.dangling);
        assert!(r.dangling[0].contains("old-note"));
        assert!(r.dangling[0].contains("inexistant"));
        assert!(r.forks.is_empty());
    }

    #[test]
    fn a_note_without_files_never_appears_in_all_files_gone() {
        let dir = fixture("no-files");
        write_feature(
            &dir,
            "no-files-declared.md",
            "---\nfeature: No Files Declared\nslug: no-files-declared\n---\n\n# No Files Declared\n",
        );
        write_feature(
            &dir,
            "files-really-gone.md",
            "---\nfeature: Files Really Gone\nslug: files-really-gone\nfiles: [src/does/not/exist.rs]\n---\n\n# Files Really Gone\n",
        );

        // `dir` itself has no `src/does/not/exist.rs`, so the declared-files
        // note is a genuine hit; the files-less note must never be reported,
        // no matter what the workspace looks like.
        let r = report(&dir, &dir);
        assert!(!r.all_files_gone.contains(&"no-files-declared".to_string()));
        assert!(r.all_files_gone.contains(&"files-really-gone".to_string()));
    }

    #[test]
    fn report_never_writes_to_disk() {
        let dir = fixture("read-only");
        write_feature(
            &dir,
            "auto-exclusion.md",
            "---\nfeature: Auto Exclusion\nslug: auto-exclusion\n---\n\n# Auto Exclusion\n",
        );
        write_feature(
            &dir,
            "autoexclusion.md",
            "---\nfeature: Auto Exclusion\nslug: autoexclusion\nsuperseded_by: inexistant\nstatus: deprecated\n---\n\n# Auto Exclusion\n",
        );

        let features = crate::knowledge::features_dir(&dir);
        let before: Vec<(std::path::PathBuf, Vec<u8>)> = std::fs::read_dir(&features)
            .unwrap()
            .flatten()
            .map(|e| (e.path(), std::fs::read(e.path()).unwrap()))
            .collect();

        let _ = report(&dir, &dir);

        let after: Vec<(std::path::PathBuf, Vec<u8>)> = std::fs::read_dir(&features)
            .unwrap()
            .flatten()
            .map(|e| (e.path(), std::fs::read(e.path()).unwrap()))
            .collect();

        assert_eq!(
            before.len(),
            after.len(),
            "report() must not create or delete any file"
        );
        for (path, bytes) in &before {
            let still = after.iter().find(|(p, _)| p == path);
            assert_eq!(
                still.map(|(_, b)| b),
                Some(bytes),
                "content of {path:?} must be byte-for-byte unchanged after report()"
            );
        }
    }
}
