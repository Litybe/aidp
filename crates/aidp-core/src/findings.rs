//! findings.rs — structs mirroring `schema/findings.schema.json`, and the
//! deterministic merge of N axis `findings.json` files into ONE canonical
//! `findings.json` (Lot 3, Capitalisation, C6).
//!
//! ## Why this exists
//!
//! `templates/review.yaml` runs four independent reviewer axes, then a fifth
//! "synthesis" stage that used to be asked — in prose — to combine their four
//! `findings.json` into the canonical one the correction loop and `pr` stage
//! read (`signals.has_blocking_findings`). That aggregation — union, max of
//! severity, logical OR — is a purely mechanical operation that was being
//! delegated to a fifth model call: a `blocking` finding from the security axis
//! could be silently reclassified `major` or dropped at synthesis, with nothing
//! to notice. This module makes the aggregation a real function instead: [`merge`].
//!
//! ## Draft choice: draft-07, not 2020-12
//!
//! `schema/findings.schema.json` declares `"$schema":
//! ".../draft/2020-12/schema"`, but every other schema this workspace compiles
//! with the `jsonschema` crate (`plan.rs`, `pipeline.rs`, `run.rs`) forces
//! `Draft::Draft7`. Rather than let this be the one schema compiled under a
//! different draft — which would make "which draft does this repo's JSON
//! Schema use" a per-file question instead of a workspace answer — the
//! `$schema` string itself is changed to draft-07 (see the diff on
//! `schema/findings.schema.json`) and this module forces `Draft::Draft7` like
//! the other three call sites. Nothing in this schema needs 2020-12: the only
//! feature the `checks[]` addition uses is `if`/`then`, which draft-07 has had
//! since its own release.
//!
//! ## The five merge rules
//!
//! 1. Output finding id = `F-<axis>-<source id without its "F-" prefix>` — so
//!    two axes that both emitted `F-1` don't collide.
//! 2. `severity_counts` is **recomputed** from the merged `findings[]`, never
//!    copied from an input; `has_blocking_findings = counts.blocking > 0`.
//! 3. `checks[]` are merged by `id` across axes, keeping the worst status in
//!    the order `fail > skip > pass` — an axis that actually failed a control
//!    always outranks one that skipped or passed it.
//! 4. Findings are conservatively deduplicated on `(file, line, normalized
//!    title)` — conservative because a finding missing either `file` or `line`
//!    is never folded into anything (no anchor strong enough to be sure it's
//!    the same issue) — keeping the finding with the MAXIMAL severity. Every
//!    fold (kept vs dropped) is named in the merge's own log, never silent.
//! 5. A broken entry is handled two different ways depending on WHY it broke
//!    (LOC-42, Lot 3 Capitalisation):
//!    - **Degrade just that one axis** — 0 findings, a synthetic
//!      `C-<axis>-harness-gap` check (`status: "skip"`, `harness_gap` naming
//!      the exact read/parse/schema error) and a `merge_log` line naming the
//!      axis — when its `findings.json` is absent, unreadable, not valid
//!      JSON, or fails `findings.schema.json`. The axis produced nothing
//!      coherent to disagree with, which is an honest coverage gap the
//!      synthesis stage can act on (see [`MergeOutcome::degraded_axes`]),
//!      not a reason to strand the N-1 other axes that DID produce a
//!      verdict.
//!    - **Hard-fail the whole call**, naming the offending entry (its axis
//!      label AND its path), when an axis's own `severity_counts` does not
//!      match its own `findings[]` — an axis that lied about its own count
//!      must never be laundered into "nothing to report" by degrading it
//!      away; that would hide the lie instead of surfacing it.

use crate::clock::{Clock, SystemClock};
use crate::models::finding_location::FindingLocation;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const SCHEMA: &str = include_str!("../../../schema/findings.schema.json");

// ---------------------------------------------------------------------------
// Structs mirroring findings.schema.json
// ---------------------------------------------------------------------------

/// How serious a [`Finding`] is. Declared low-to-high so `Ord`/`max` picks the
/// worst one, matching rule 4 (dedup keeps the maximal severity).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Minor,
    Major,
    Blocking,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Info => "info",
            Severity::Minor => "minor",
            Severity::Major => "major",
            Severity::Blocking => "blocking",
        }
    }
}

/// One review finding. `category` is left as `String` — its exact enum is
/// `findings.schema.json`'s job to enforce; nothing in the merge logic branches
/// on which category a finding carries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    pub id: String,
    pub severity: Severity,
    pub category: String,
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default)]
    pub line: Option<i64>,
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub suggestion: Option<String>,
    #[serde(default)]
    pub related_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<FindingLocation>,
}

/// A control's outcome. Declared low-to-high gravity (`fail` outranks
/// everything) so `Ord`/`max` implements rule 3 directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Pass,
    Skip,
    Fail,
}

/// One three-state control (`agents/reviewer.md`'s "what did you actually
/// check" contract). `skip` MUST carry `harness_gap`; `fail` MUST carry
/// `finding_ids` — both enforced by `findings.schema.json`'s `if`/`then`, not
/// re-checked here (see [`validate_json`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Check {
    pub id: String,
    pub name: String,
    pub status: CheckStatus,
    #[serde(default)]
    pub harness_gap: Option<String>,
    #[serde(default)]
    pub evidence: Option<String>,
    #[serde(default)]
    pub finding_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeverityCounts {
    pub blocking: u32,
    pub major: u32,
    pub minor: u32,
    pub info: u32,
}

/// `scores.*` values per `code-audit-rubric` (`{"correctness":4,
/// "correctness_note":"..."}`) — either an integer score or a string note.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ScoreValue {
    Int(i64),
    Str(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Findings {
    #[serde(default = "default_schema_version")]
    pub schema_version: i64,
    pub run_id: String,
    pub iter: u32,
    pub generated_at: String,
    pub severity_counts: SeverityCounts,
    pub has_blocking_findings: bool,
    pub findings: Vec<Finding>,
    /// `#[serde(default)]` here (not a hard requirement) is deliberate: the
    /// SCHEMA requires `checks` (C6), but this Rust-level struct is also used
    /// by `obsidian.rs` for a best-effort display of pre-C6 findings.json
    /// files that predate the field — those must still deserialize, just with
    /// an empty (and honestly empty, not fabricated) checks list.
    #[serde(default)]
    pub checks: Vec<Check>,
    #[serde(default)]
    pub scores: BTreeMap<String, ScoreValue>,
}

fn default_schema_version() -> i64 {
    1
}

// ---------------------------------------------------------------------------
// Loading + validation
// ---------------------------------------------------------------------------

fn validate_json(instance: &serde_json::Value) -> anyhow::Result<()> {
    let schema: serde_json::Value = serde_json::from_str(SCHEMA)?;
    let compiled = jsonschema::JSONSchema::options()
        .with_draft(jsonschema::Draft::Draft7)
        .compile(&schema)
        .map_err(|e| anyhow::anyhow!("findings schema compile: {e}"))?;
    if let Err(errors) = compiled.validate(instance) {
        let msgs: Vec<String> = errors
            .map(|e| format!("{}: {}", e.instance_path, e))
            .collect();
        anyhow::bail!(
            "findings violate schema/findings.schema.json:\n  {}",
            msgs.join("\n  ")
        );
    }
    Ok(())
}

/// F-9 (Lot 3, Capitalisation): the one place that counts a `&[Finding]` slice
/// by severity — [`validate_counts`] (cross-checking a loaded document) and
/// [`recompute_counts`] (computing one for a freshly merged document) used to
/// each carry their own identical 4-arm match; this is now the single source
/// both call.
fn tally(findings: &[Finding]) -> SeverityCounts {
    let mut c = SeverityCounts::default();
    for finding in findings {
        match finding.severity {
            Severity::Blocking => c.blocking += 1,
            Severity::Major => c.major += 1,
            Severity::Minor => c.minor += 1,
            Severity::Info => c.info += 1,
        }
    }
    c
}

/// A `severity_counts` that does not match `findings[]` means the axis
/// misreported its own tally — rule 5's third failure mode. Not expressible in
/// JSON Schema (it is a cross-field arithmetic constraint), so checked here.
fn validate_counts(f: &Findings) -> anyhow::Result<()> {
    let computed = tally(&f.findings);
    if computed != f.severity_counts {
        anyhow::bail!(
            "severity_counts {{blocking:{}, major:{}, minor:{}, info:{}}} does not match the {} finding(s) actually listed (recomputed: {{blocking:{}, major:{}, minor:{}, info:{}}}) — this axis misreported its own count",
            f.severity_counts.blocking, f.severity_counts.major, f.severity_counts.minor, f.severity_counts.info,
            f.findings.len(),
            computed.blocking, computed.major, computed.minor, computed.info,
        );
    }
    Ok(())
}

/// Why [`parse`]/[`load`] failed — [`merge`]'s rule 5 treats these two
/// categories very differently, so it is a typed enum rather than a flat
/// `anyhow::Error`: pattern-matching on it (instead of, say, sniffing the
/// message text) is what lets `merge` degrade one without ever accidentally
/// degrading the other.
///
/// - `Unreadable`: the document never cohered — missing file, unreadable,
///   invalid JSON, or a `findings.schema.json` violation. Nothing to
///   disagree with; `merge` treats this as an honest harness gap.
/// - `CountMismatch`: the document parsed and validated fine, but its own
///   `severity_counts` contradicts its own `findings[]` — the axis actively
///   misreported its tally. `merge` must never launder this into "nothing to
///   report".
#[derive(Debug)]
pub enum LoadError {
    Unreadable(String),
    CountMismatch(String),
}

impl LoadError {
    fn message(&self) -> &str {
        match self {
            LoadError::Unreadable(m) | LoadError::CountMismatch(m) => m,
        }
    }

    /// Prefix the message with `path`, the way `load` has always done,
    /// while preserving which of the two categories this is.
    fn with_path_prefix(self, path: &Path) -> LoadError {
        let prefixed = format!("{} — {}", path.display(), self.message());
        match self {
            LoadError::Unreadable(_) => LoadError::Unreadable(prefixed),
            LoadError::CountMismatch(_) => LoadError::CountMismatch(prefixed),
        }
    }
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for LoadError {}

/// Read and validate one `findings.json` document from memory (schema, then
/// the count cross-check). Mirrors `plan::parse`'s shape.
pub fn parse(text: &str) -> Result<Findings, LoadError> {
    let json: serde_json::Value = serde_json::from_str(text)
        .map_err(|e| LoadError::Unreadable(format!("findings is not valid JSON: {e}")))?;
    validate_json(&json).map_err(|e| LoadError::Unreadable(e.to_string()))?;
    let findings: Findings = serde_json::from_value(json).map_err(|e| {
        LoadError::Unreadable(format!(
            "findings does not match findings.schema.json's shape: {e}"
        ))
    })?;
    validate_counts(&findings).map_err(|e| LoadError::CountMismatch(e.to_string()))?;
    Ok(findings)
}

/// Read and validate `path`. Mirrors `plan::load`'s shape.
pub fn load(path: &Path) -> Result<Findings, LoadError> {
    let text = crate::text::read_text_lenient(path)
        .map_err(|e| LoadError::Unreadable(format!("read findings {}: {e}", path.display())))?;
    parse(&text).map_err(|e| e.with_path_prefix(path))
}

// ---------------------------------------------------------------------------
// Merge
// ---------------------------------------------------------------------------

/// One axis to fold into the merge: its label (used both for the output
/// finding-id prefix and for naming it in an error) and the path to its
/// `findings.json`.
#[derive(Debug, Clone)]
pub struct MergeInput {
    pub label: String,
    pub path: PathBuf,
}

/// The merge's result: the canonical, schema-valid `Findings` PLUS the
/// human-readable log of every dedup fold — kept separate from `Findings`
/// itself because `findings.schema.json` is `additionalProperties: false` and
/// does not model a merge log; the log is the CLI command's own output, not
/// part of the artifact.
#[derive(Debug, Clone, Serialize)]
pub struct MergeOutcome {
    pub findings: Findings,
    pub merge_log: Vec<String>,
    /// Axis labels [`merge`] degraded rather than failed the call on (rule
    /// 5's `Unreadable` branch) — empty when every input's `findings.json`
    /// loaded and validated cleanly. Named explicitly (not just left to
    /// `merge_log` prose) so a caller like `findings_cmd` can surface it as
    /// its own structured field instead of a synthesis stage having to
    /// re-derive it from free text.
    pub degraded_axes: Vec<String>,
}

fn sanitize_axis(label: &str) -> String {
    let s: String = label
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect();
    if s.is_empty() {
        "axis".to_string()
    } else {
        s
    }
}

/// Case/whitespace-insensitive title comparison for rule 4's dedup key.
fn normalize_title(title: &str) -> String {
    title
        .trim()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn strip_f_prefix(id: &str) -> &str {
    id.strip_prefix("F-").unwrap_or(id)
}

/// Merge N axis `findings.json` files into one canonical [`Findings`], per the
/// five rules in this module's doc comment. `inputs` are folded in the given
/// order, so re-running the same command on the same files is deterministic.
pub fn merge(inputs: &[MergeInput]) -> anyhow::Result<MergeOutcome> {
    merge_with(inputs, &SystemClock)
}

/// [`merge`], taking its `generated_at` timestamp from `clock` instead of
/// reading the wall clock directly.
pub fn merge_with(inputs: &[MergeInput], clock: &dyn Clock) -> anyhow::Result<MergeOutcome> {
    if inputs.is_empty() {
        anyhow::bail!("findings merge needs at least one input");
    }
    let mut merge_log: Vec<String> = Vec::new();
    let mut findings_out: Vec<Finding> = Vec::new();
    // Conservative dedup key: (file, line, normalized title). A finding
    // without a concrete file+line never enters this index — see rule 4.
    let mut dedup_index: BTreeMap<(String, i64, String), usize> = BTreeMap::new();
    // check id -> every (axis, check) that reported on it, in input order.
    let mut checks_by_id: BTreeMap<String, Vec<(String, Check)>> = BTreeMap::new();
    let mut run_id: Option<String> = None;
    let mut iter: Option<u32> = None;
    let mut degraded_axes: Vec<String> = Vec::new();

    for input in inputs {
        let axis = sanitize_axis(&input.label);
        let entry = match load(&input.path) {
            Ok(entry) => entry,
            // Rule 5, hard-fail branch: this axis parsed and validated fine
            // but lied about its own tally — never launder that into
            // "nothing to report" by degrading it away like the branch below.
            Err(LoadError::CountMismatch(msg)) => {
                anyhow::bail!(
                    "findings entry `{}` ({}): {msg}",
                    input.label,
                    input.path.display()
                );
            }
            // Rule 5, degrade branch: this axis never produced anything
            // coherent (missing/unreadable/invalid/schema-violating) — fold
            // it into a synthetic harness-gap check instead of stranding the
            // other N-1 axes that DID produce a real verdict.
            Err(LoadError::Unreadable(msg)) => {
                let detail = format!(
                    "findings entry `{}` ({}): {msg}",
                    input.label,
                    input.path.display()
                );
                merge_log.push(format!(
                    "harness gap: axis `{}` degraded (findings.json unreadable) — {detail}",
                    input.label
                ));
                degraded_axes.push(input.label.clone());
                let check_id = format!("C-{axis}-harness-gap");
                checks_by_id.entry(check_id.clone()).or_default().push((
                    input.label.clone(),
                    Check {
                        id: check_id,
                        name: format!("{} findings.json readable", input.label),
                        status: CheckStatus::Skip,
                        harness_gap: Some(detail),
                        evidence: None,
                        finding_ids: Vec::new(),
                    },
                ));
                continue;
            }
        };

        match &run_id {
            None => run_id = Some(entry.run_id.clone()),
            Some(r) if *r != entry.run_id => merge_log.push(format!(
                "note: axis `{}` carries run_id `{}`, differing from `{}` — kept the first seen",
                input.label, entry.run_id, r
            )),
            _ => {}
        }
        match iter {
            None => iter = Some(entry.iter),
            Some(i) if i != entry.iter => merge_log.push(format!(
                "note: axis `{}` carries iter {}, differing from {} — kept the first seen",
                input.label, entry.iter, i
            )),
            _ => {}
        }

        for finding in entry.findings {
            let out_id = format!("F-{axis}-{}", strip_f_prefix(&finding.id));
            let mut remapped = finding;
            remapped.id = out_id.clone();

            let key = match (&remapped.file, remapped.line) {
                (Some(file), Some(line)) => {
                    Some((file.clone(), line, normalize_title(&remapped.title)))
                }
                _ => None,
            };

            let Some(key) = key else {
                findings_out.push(remapped);
                continue;
            };
            if let Some(&idx) = dedup_index.get(&key) {
                let existing = &findings_out[idx];
                if remapped.severity > existing.severity {
                    merge_log.push(format!(
                        "dedup: `{}` supersedes `{}` at {}:{} (\"{}\") — kept the higher severity ({} > {})",
                        out_id, existing.id, key.0, key.1, existing.title,
                        remapped.severity.as_str(), existing.severity.as_str(),
                    ));
                    findings_out[idx] = remapped;
                } else {
                    merge_log.push(format!(
                        "dedup: `{}` dropped as a duplicate of `{}` at {}:{} (\"{}\") — severity {} <= {}",
                        out_id, existing.id, key.0, key.1, existing.title,
                        remapped.severity.as_str(), existing.severity.as_str(),
                    ));
                }
                continue;
            }
            dedup_index.insert(key, findings_out.len());
            findings_out.push(remapped);
        }

        for check in entry.checks {
            // Remap this axis's own (unprefixed) finding_ids through the same
            // rule-1 prefixing so they still point at real ids post-merge.
            let mut c = check;
            c.finding_ids = c
                .finding_ids
                .iter()
                .map(|fid| format!("F-{axis}-{}", strip_f_prefix(fid)))
                .collect();
            checks_by_id
                .entry(c.id.clone())
                .or_default()
                .push((input.label.clone(), c));
        }
    }

    let merged_checks = merge_checks(checks_by_id);
    let counts = recompute_counts(&findings_out);
    let has_blocking_findings = counts.blocking > 0;

    let out = Findings {
        schema_version: 1,
        run_id: run_id.unwrap_or_default(),
        iter: iter.unwrap_or(0),
        generated_at: clock.now_rfc3339(),
        severity_counts: counts,
        has_blocking_findings,
        findings: findings_out,
        checks: merged_checks,
        // The five rules don't cover `scores` merging — left empty rather than
        // guessing a policy nobody asked for.
        scores: BTreeMap::new(),
    };
    Ok(MergeOutcome {
        findings: out,
        merge_log,
        degraded_axes,
    })
}

/// F-9: a pure alias for [`tally`] — kept as its own function because it has
/// call sites distinct from [`validate_counts`]'s, not because it needs its
/// own logic.
fn recompute_counts(findings: &[Finding]) -> SeverityCounts {
    tally(findings)
}

/// Rule 3: one merged [`Check`] per id, worst status wins (`fail > skip >
/// pass`). `harness_gap`/`evidence` are joined across every reporting axis
/// (never just the winning one — a gap an axis hit is worth keeping visible
/// even if another axis's `fail` outranks it); `finding_ids` is the union,
/// already remapped to the merged findings' ids.
fn merge_checks(by_id: BTreeMap<String, Vec<(String, Check)>>) -> Vec<Check> {
    by_id
        .into_iter()
        .map(|(id, group)| {
            let winning_status = group.iter().map(|(_, c)| c.status).max().unwrap();
            let name = group
                .iter()
                .find(|(_, c)| c.status == winning_status)
                .map(|(_, c)| c.name.clone())
                .unwrap_or_else(|| group[0].1.name.clone());

            let mut harness_gaps: Vec<String> = Vec::new();
            let mut evidences: Vec<String> = Vec::new();
            let mut finding_ids: Vec<String> = Vec::new();
            for (axis, c) in &group {
                if let Some(g) = &c.harness_gap {
                    let line = format!("{axis}: {g}");
                    if !harness_gaps.contains(&line) {
                        harness_gaps.push(line);
                    }
                }
                if let Some(e) = &c.evidence {
                    let line = format!("{axis}: {e}");
                    if !evidences.contains(&line) {
                        evidences.push(line);
                    }
                }
                for fid in &c.finding_ids {
                    if !finding_ids.contains(fid) {
                        finding_ids.push(fid.clone());
                    }
                }
            }
            Check {
                id,
                name,
                status: winning_status,
                harness_gap: (!harness_gaps.is_empty()).then(|| harness_gaps.join("; ")),
                evidence: (!evidences.is_empty()).then(|| evidences.join("; ")),
                finding_ids,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use aidp_testkit::TempWorkspace;

    fn fixture(name: &str) -> PathBuf {
        PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/findings"
        ))
        .join(name)
    }

    fn input(label: &str, name: &str) -> MergeInput {
        MergeInput {
            label: label.to_string(),
            path: fixture(name),
        }
    }

    fn finding_of(id: &str, severity: Severity) -> Finding {
        Finding {
            id: id.to_string(),
            severity,
            category: "correctness".to_string(),
            file: None,
            line: None,
            title: "t".to_string(),
            description: None,
            suggestion: None,
            related_ids: Vec::new(),
            location: None,
        }
    }

    // F-9: `validate_counts` and `recompute_counts` both delegate to `tally` —
    // one finding of each severity, all four counters must land right.
    #[test]
    fn tally_counts_every_severity_arm() {
        let findings = vec![
            finding_of("F-1", Severity::Blocking),
            finding_of("F-2", Severity::Major),
            finding_of("F-3", Severity::Minor),
            finding_of("F-4", Severity::Info),
        ];
        let counts = tally(&findings);
        assert_eq!(
            counts,
            SeverityCounts {
                blocking: 1,
                major: 1,
                minor: 1,
                info: 1,
            }
        );
    }

    #[test]
    fn it_merge_four_axes_one_blocking_sets_has_blocking_findings() {
        // 4 axes: correctness (1 blocking), security (0, all skip), perf (1
        // blocking — a duplicate collapsed by the dedup test below), and a
        // valid stand-in maintainability axis with self-consistent counts.
        // `maintainability-bad-counts.json` is deliberately invalid and is
        // exercised by its own error test, not this success path.
        let dir = std::env::temp_dir().join(format!("aidp-findings-4ax-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let maint_ok = dir.join("maintainability-ok.json");
        std::fs::write(
            &maint_ok,
            serde_json::json!({
                "schema_version": 1,
                "run_id": "run-c6-fixture",
                "iter": 0,
                "generated_at": "2026-09-09T10:00:00Z",
                "severity_counts": { "blocking": 0, "major": 0, "minor": 1, "info": 0 },
                "has_blocking_findings": false,
                "findings": [
                    { "id": "F-1", "severity": "minor", "category": "maintainability", "file": "src/bar.rs", "line": 5, "title": "Unclear name" }
                ],
                "checks": []
            })
            .to_string(),
        )
        .unwrap();

        let outcome = merge(&[
            input("correctness", "correctness.json"),
            input("security", "security.json"),
            input("perf", "perf.json"),
            MergeInput {
                label: "maintainability".into(),
                path: maint_ok,
            },
        ])
        .unwrap();
        assert!(outcome.findings.has_blocking_findings);
        assert!(outcome.findings.severity_counts.blocking >= 1);
        // 4 distinct axes contributed — sanity check the merge actually ran
        // over all of them rather than short-circuiting.
        assert!(outcome
            .findings
            .findings
            .iter()
            .any(|f| f.id.starts_with("F-maintainability-")));
    }

    /// LOC-42: a single input is ALSO the whole merge, so this is the
    /// smallest possible reproduction of "one degraded axis must not strand
    /// the call" — see `loc18_missing_axis_degrades_instead_of_stranding_the_whole_merge`
    /// below for the exact 4-axis incident shape.
    #[test]
    fn it_missing_file_degrades_the_axis_instead_of_failing() {
        let outcome = merge(&[input("security", "does-not-exist.json")])
            .expect("a missing axis must degrade, never fail the whole merge");
        assert_eq!(outcome.degraded_axes, vec!["security".to_string()]);
        assert!(
            outcome
                .merge_log
                .iter()
                .any(|l| l.contains("security") && l.contains("does-not-exist.json")),
            "merge_log must name the axis and the path: {:?}",
            outcome.merge_log
        );
        let check = outcome
            .findings
            .checks
            .iter()
            .find(|c| c.id == "C-security-harness-gap")
            .expect("a synthetic harness-gap check must stand in for the degraded axis");
        assert_eq!(check.status, CheckStatus::Skip);
        assert!(check
            .harness_gap
            .as_deref()
            .is_some_and(|g| g.contains("does-not-exist.json")));
    }

    #[test]
    fn it_bad_json_degrades_the_axis_instead_of_failing() {
        let ws = TempWorkspace::new("badjson");
        let dir = ws.path().to_path_buf();
        let bad = dir.join("bad.json");
        std::fs::write(&bad, "{ not json").unwrap();
        let outcome = merge(&[MergeInput {
            label: "perf".into(),
            path: bad.clone(),
        }])
        .expect("invalid JSON must degrade the axis, never fail the whole merge");
        assert_eq!(outcome.degraded_axes, vec!["perf".to_string()]);
        let check = outcome
            .findings
            .checks
            .iter()
            .find(|c| c.id == "C-perf-harness-gap")
            .expect("a synthetic harness-gap check must stand in for the degraded axis");
        assert_eq!(check.status, CheckStatus::Skip);
        assert!(check.harness_gap.is_some());
    }

    // Reproduces the exact LOC-18 incident: a `--label perf=<path>` whose
    // file was never written (the review_perf axis replied `completed` with
    // `outputs: {}` and `signals.has_harness_gap: true`, and no findings.json
    // ever existed at the path the synthesis stage was told to merge).
    // Before this fix, `merge` propagated `load`'s error for ANY input with
    // `?`, which stranded the entire synthesis stage even though
    // correctness/security/maintainability each produced a real, readable
    // verdict. After the fix: Ok, a schema-valid findings.json, and `perf`
    // named as the one degraded axis — never a crash.
    #[test]
    fn it_loc18_missing_axis_degrades_instead_of_stranding_the_whole_merge() {
        let outcome = merge(&[
            input("correctness", "correctness.json"),
            input("security", "security.json"),
            MergeInput {
                label: "perf".into(),
                path: fixture("does-not-exist.json"),
            },
            // Any schema-valid file works as the fourth "valid" axis for
            // this test's purpose — reusing security.json's content under a
            // different label rather than adding a redundant fixture file.
            input("maintainability", "security.json"),
        ])
        .expect("a missing axis must degrade, never strand the other three");

        assert_eq!(
            outcome.degraded_axes,
            vec!["perf".to_string()],
            "exactly the missing axis must be named as degraded"
        );
        assert!(
            outcome
                .merge_log
                .iter()
                .any(|l| l.contains("perf") && l.contains("does-not-exist.json")),
            "merge_log must name the degraded axis and the read error, not just that one exists: {:?}",
            outcome.merge_log
        );

        let harness_check = outcome
            .findings
            .checks
            .iter()
            .find(|c| c.id == "C-perf-harness-gap")
            .expect("a synthetic harness-gap check must stand in for the degraded axis");
        assert_eq!(harness_check.status, CheckStatus::Skip);
        assert!(
            harness_check
                .harness_gap
                .as_deref()
                .is_some_and(|g| g.contains("does-not-exist.json")),
            "harness_gap must cite the real read error: {:?}",
            harness_check.harness_gap
        );

        // The other three axes are untouched: their findings still made it through.
        assert!(outcome
            .findings
            .findings
            .iter()
            .any(|f| f.id.starts_with("F-correctness-")));

        // The produced document is itself schema-valid — never a malformed
        // "recovery" artifact that would just fail one step later.
        let text = serde_json::to_string(&outcome.findings).unwrap();
        parse(&text).expect("the merged output must itself validate against findings.schema.json");
    }

    #[test]
    fn it_mismatched_severity_counts_errors_naming_the_entry() {
        let err = merge(&[input("maintainability", "maintainability-bad-counts.json")])
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("maintainability"),
            "error must name the axis: {err}"
        );
        assert!(
            err.contains("misreported"),
            "error must explain the count mismatch: {err}"
        );
    }

    #[test]
    fn it_duplicate_finding_across_axes_collapses_to_one_with_max_severity_and_is_logged() {
        // perf.json's one finding is a same-(file,line,title) duplicate of one
        // of correctness.json's two findings (see fixtures).
        let outcome = merge(&[
            input("correctness", "correctness.json"),
            input("perf", "perf.json"),
        ])
        .unwrap();
        let dup_titles: Vec<&str> = outcome
            .findings
            .findings
            .iter()
            .map(|f| f.title.as_str())
            .collect();
        // Only ONE surviving finding for the duplicated (file, line, title).
        let dup_count = dup_titles
            .iter()
            .filter(|t| t.eq_ignore_ascii_case("n+1 query in the hot loop"))
            .count();
        assert_eq!(
            dup_count, 1,
            "duplicate must collapse to a single finding: {dup_titles:?}"
        );
        assert!(
            outcome.merge_log.iter().any(|l| l.contains("dedup:")),
            "the fold must be logged, never silent: {:?}",
            outcome.merge_log
        );
    }

    #[test]
    fn check_missing_harness_gap_on_skip_is_rejected_by_schema() {
        let bad = serde_json::json!({
            "schema_version": 1,
            "run_id": "r1",
            "iter": 0,
            "generated_at": "2026-01-01T00:00:00Z",
            "severity_counts": { "blocking": 0, "major": 0, "minor": 0, "info": 0 },
            "has_blocking_findings": false,
            "findings": [],
            "checks": [
                { "id": "C-diff-read", "name": "Read the diff", "status": "skip" }
            ]
        });
        let err = parse(&bad.to_string()).unwrap_err().to_string();
        assert!(
            err.contains("harness_gap") || err.contains("schema"),
            "a skip without harness_gap must be rejected by the schema: {err}"
        );
    }
}
