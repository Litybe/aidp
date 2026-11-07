//! C7 (Lot 3, Capitalisation): a verifier for what a stage actually PRODUCED.
//!
//! The runtime's JSON Schema validation on a sub-agent's reply envelope checks
//! a *shape* — that `outputs` is a map of strings, that `status` is one of the
//! enum values. It has never once opened the files those `outputs` point at.
//! A sub-agent that returns `status: "completed"` with an `outputs.plan`
//! pointing at a truncated file, a refusal message, or invalid JSON passes
//! schema validation cleanly and sails through review.
//!
//! [`verify_outputs`] is that missing second look: cheap, syntactic,
//! content-blind checks against every file a stage's envelope claims to have
//! written. It is deliberately **not** a gate — see the module-level framing in
//! the task spec ("ça ne bloque rien"): its findings land in
//! `StageState.verification` as annotation for a human (or a future run) to
//! read, never as a reason to fail the run that produced them. A stage that
//! genuinely failed already shows up as `Cancel`; this catches the quieter
//! failure mode where a stage *looks* done and is not.

use crate::run::StageState;
use std::path::Path;
use std::path::PathBuf;

/// One output file's findings. Only ever constructed for a file that raised at
/// least one signal — [`verify_outputs`]'s return value doubles as "is there
/// anything here worth a human's attention", so a clean file earns no entry
/// rather than a `signals: []` one.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FileVerdict {
    pub path: PathBuf,
    pub signals: Vec<String>,
}

/// Below this many bytes of trimmed content, a declared output reads as
/// suspiciously thin for anything but the emptiest of stages.
const MIN_LEN_BYTES: usize = 20;

/// How many leading characters of a file's content the refusal-pattern check
/// scans. A refusal is the file's opening move when a model produces one
/// instead of doing the work — bounding the search keeps the check cheap and
/// immune to a later paragraph that merely *quotes* a refusal as an example.
const REFUSAL_SCAN_CHARS: usize = 240;

/// A file whose dominant non-blank line accounts for at least this fraction of
/// its non-blank lines reads as a stuck / looping generation rather than real
/// content.
const DEGENERATE_MIN_RATIO: f32 = 0.5;

/// ...but only once there are enough lines for that ratio to mean anything —
/// a one-line file is trivially "100% one line" and must never trip this.
const DEGENERATE_MIN_LINES: usize = 5;

fn refusal_regex() -> &'static regex::Regex {
    static RE: once_cell::sync::OnceCell<regex::Regex> = once_cell::sync::OnceCell::new();
    RE.get_or_init(|| {
        regex::Regex::new(
            r#"(?i)(as an ai( language model)?\b|i cannot (help|assist|complete|do that|comply)|i can.t (help|assist|complete|do that|comply)|i'm sorry,? but|i am sorry,? but|as a language model|en tant qu'ia\b|je ne peux pas (vous aider|le faire|r[ée]pondre|proc[ée]der)|je suis d[ée]sol[ée],? mais|en tant qu'assistant\b)"#,
        )
        .unwrap()
    })
}

/// Applies every syntactic check to every file `stage.outputs` references.
/// Non-blocking by construction: a missing file, an unreadable one, or a
/// signal firing on a real one never panics and never aborts — the caller
/// (`run::reconcile`) folds the result straight into `StageState.verification`
/// as annotation, never as a gate.
pub fn verify_outputs(stage: &StageState) -> Vec<FileVerdict> {
    stage
        .outputs
        .values()
        .filter_map(|path| {
            if !is_within_stage_dir(&stage.stage_dir, path) {
                return Some(FileVerdict {
                    path: path.clone(),
                    signals: vec!["out-of-scope: outside stage_dir, not scanned".to_string()],
                });
            }
            let signals = verify_one(path);
            if signals.is_empty() {
                None
            } else {
                Some(FileVerdict {
                    path: path.clone(),
                    signals,
                })
            }
        })
        .collect()
}

/// Best-effort containment check: canonicalize both sides when possible (a
/// symlink or a `..` segment must not fool this), and fall back to a lexical
/// `starts_with` when the output path does not exist yet (`verify_one`'s own
/// "missing-or-unreadable" case) — a file that cannot be canonicalized must
/// still be confined, not silently passed through.
fn is_within_stage_dir(stage_dir: &Path, path: &Path) -> bool {
    match (stage_dir.canonicalize(), path.canonicalize()) {
        (Ok(root), Ok(p)) => p.starts_with(root),
        _ => path.starts_with(stage_dir),
    }
}

fn verify_one(path: &PathBuf) -> Vec<String> {
    let mut signals = Vec::new();

    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            signals.push(format!("missing-or-unreadable: {e}"));
            return signals;
        }
    };
    let content = String::from_utf8_lossy(&bytes).into_owned();
    let trimmed = content.trim();

    if trimmed.is_empty() {
        signals.push("empty-file".to_string());
        // Nothing else is worth checking on a file with no content at all.
        return signals;
    }
    if trimmed.len() < MIN_LEN_BYTES {
        signals.push(format!("too-short ({} bytes)", trimmed.len()));
    }

    // An odd number of ``` fences means one was opened and never closed — the
    // classic shape of a response truncated mid-code-block.
    let fence_count = content.matches("```").count();
    if !fence_count.is_multiple_of(2) {
        signals.push("unbalanced-code-fence".to_string());
    }

    let head: String = content.chars().take(REFUSAL_SCAN_CHARS).collect();
    if refusal_regex().is_match(&head) {
        signals.push("refusal-pattern".to_string());
    }

    if let Some(ratio) = degenerate_repetition_ratio(&content) {
        signals.push(format!(
            "degenerate-repetition ({:.0}% one line)",
            ratio * 100.0
        ));
    }

    let is_json = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("json"))
        .unwrap_or(false);
    if is_json {
        if let Err(e) = serde_json::from_str::<serde_json::Value>(&content) {
            signals.push(format!("invalid-json: {e}"));
        }
    }

    signals
}

/// `Some(ratio)` when the file's most repeated non-blank line accounts for at
/// least [`DEGENERATE_MIN_RATIO`] of all its non-blank lines, and there are at
/// least [`DEGENERATE_MIN_LINES`] of them.
fn degenerate_repetition_ratio(content: &str) -> Option<f32> {
    let lines: Vec<&str> = content
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    if lines.len() < DEGENERATE_MIN_LINES {
        return None;
    }
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for l in &lines {
        *counts.entry(*l).or_insert(0) += 1;
    }
    let max = counts.values().copied().max().unwrap_or(0);
    let ratio = max as f32 / lines.len() as f32;
    (ratio >= DEGENERATE_MIN_RATIO).then_some(ratio)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run::StageStatus;

    use std::collections::BTreeMap;

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/verify"
        ))
        .join(name)
    }

    // Derives `stage_dir` from the output path's own parent — more faithful to
    // the real invariant anyway (an output always lives under the stage_dir of
    // ITS OWN stage) than the old hardcoded `std::env::temp_dir()`, which broke
    // for the tests below that pass a committed fixture path instead.
    fn stage_with_output(name: &str, path: PathBuf) -> StageState {
        let stage_dir = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(std::env::temp_dir);
        stage_with_output_and_dir(name, path, stage_dir)
    }

    fn stage_with_output_and_dir(name: &str, path: PathBuf, stage_dir: PathBuf) -> StageState {
        let mut outputs = BTreeMap::new();
        outputs.insert(name.to_string(), path);
        StageState {
            name: "code".into(),
            status: StageStatus::Success,
            stage_dir,
            outputs,
            ..Default::default()
        }
    }

    #[test]
    fn it_truncated_plan_reports_unbalanced_fence() {
        let stage = stage_with_output("plan", fixture("truncated-plan.md"));
        let verdicts = verify_outputs(&stage);
        assert_eq!(verdicts.len(), 1);
        assert!(
            verdicts[0]
                .signals
                .iter()
                .any(|s| s == "unbalanced-code-fence"),
            "got: {:?}",
            verdicts[0].signals
        );
    }

    #[test]
    fn it_refusal_summary_reports_refusal_pattern() {
        let stage = stage_with_output("summary", fixture("refusal-summary.md"));
        let verdicts = verify_outputs(&stage);
        assert_eq!(verdicts.len(), 1);
        assert!(
            verdicts[0].signals.iter().any(|s| s == "refusal-pattern"),
            "got: {:?}",
            verdicts[0].signals
        );
    }

    #[test]
    fn it_broken_json_reports_invalid_json() {
        let stage = stage_with_output("plan", fixture("broken.json"));
        let verdicts = verify_outputs(&stage);
        assert_eq!(verdicts.len(), 1);
        assert!(
            verdicts[0]
                .signals
                .iter()
                .any(|s| s.starts_with("invalid-json")),
            "got: {:?}",
            verdicts[0].signals
        );
    }

    #[test]
    fn a_missing_file_is_reported_not_panicked_on() {
        let stage = stage_with_output(
            "ghost",
            std::env::temp_dir().join("aidp-verify-ghost-does-not-exist.md"),
        );
        let verdicts = verify_outputs(&stage);
        assert_eq!(verdicts.len(), 1);
        assert!(verdicts[0]
            .signals
            .iter()
            .any(|s| s.starts_with("missing-or-unreadable")));
    }

    #[test]
    fn a_healthy_output_earns_no_verdict() {
        let dir = std::env::temp_dir().join(format!("aidp-verify-clean-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("summary.md");
        std::fs::write(
            &p,
            "# Summary\n\nThe handler now validates the payload before publishing \
             the event, and the new unit tests cover both the happy path and the \
             two rejection cases.\n",
        )
        .unwrap();
        let stage = stage_with_output("summary", p);
        let verdicts = verify_outputs(&stage);
        assert!(verdicts.is_empty(), "got: {verdicts:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_healthy_json_output_earns_no_verdict() {
        let dir =
            std::env::temp_dir().join(format!("aidp-verify-cleanjson-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("changed-files.json");
        std::fs::write(&p, r#"{"files": ["src/a.rs", "src/b.rs"]}"#).unwrap();
        let stage = stage_with_output("changed-files", p);
        let verdicts = verify_outputs(&stage);
        assert!(verdicts.is_empty(), "got: {verdicts:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn degenerate_repetition_is_detected() {
        let dir =
            std::env::temp_dir().join(format!("aidp-verify-degenerate-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("summary.md");
        let body = "I did the thing.\n".repeat(12);
        std::fs::write(&p, body).unwrap();
        let stage = stage_with_output("summary", p);
        let verdicts = verify_outputs(&stage);
        assert_eq!(verdicts.len(), 1);
        assert!(
            verdicts[0]
                .signals
                .iter()
                .any(|s| s.starts_with("degenerate-repetition")),
            "got: {:?}",
            verdicts[0].signals
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    /// An output escaping its own `stage_dir` is flagged `out-of-scope` and never opened.
    fn an_output_outside_stage_dir_is_flagged_without_reading_it() {
        let foreign = tempfile::tempdir().unwrap();
        let stage =
            stage_with_output_and_dir("plan", fixture("broken.json"), foreign.path().to_path_buf());
        let verdicts = verify_outputs(&stage);
        assert_eq!(verdicts.len(), 1, "got: {verdicts:?}");
        assert!(
            verdicts[0]
                .signals
                .iter()
                .any(|s| s.starts_with("out-of-scope")),
            "got: {:?}",
            verdicts[0].signals
        );
        assert!(
            !verdicts[0]
                .signals
                .iter()
                .any(|s| s.starts_with("invalid-json")),
            "verify_one must never have read the file: {:?}",
            verdicts[0].signals
        );
    }

    #[test]
    fn too_short_output_is_flagged() {
        let dir = std::env::temp_dir().join(format!("aidp-verify-short-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("summary.md");
        std::fs::write(&p, "ok").unwrap();
        let stage = stage_with_output("summary", p);
        let verdicts = verify_outputs(&stage);
        assert_eq!(verdicts.len(), 1);
        assert!(verdicts[0]
            .signals
            .iter()
            .any(|s| s.starts_with("too-short")));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
