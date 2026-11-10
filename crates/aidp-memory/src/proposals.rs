//! Ingestion of `memory-proposals.json` files written by pipeline stages.

use crate::error::{MemoryError, Result};
use crate::manager::{validate_text, MemoryManager};
use crate::models::constants::limits::{
    DEFAULT_CONFIDENCE, INGEST_MAX_DEPTH, MAX_PROPOSALS_PER_FILE,
};
use crate::models::constants::paths::PROPOSALS_FILE;
use crate::models::constants::proposal_fields::{ISSUE_SEGMENT_PATTERN, RUN_SEGMENT_PATTERN};
use crate::models::enums::{MemoryKind, MemoryStatus, RefType, Route};
use crate::models::ingest_report::{IngestRejection, IngestReport};
use crate::models::memory_ref::MemoryRef;
use crate::models::memory_scope::MemoryScope;
use crate::models::new_memory::NewMemory;
use crate::proposal_shape::canonicalize_document;
use regex::Regex;
use serde::Deserialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use walkdir::WalkDir;

static ISSUE_SEGMENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(ISSUE_SEGMENT_PATTERN).expect("issue segment pattern"));
static RUN_SEGMENT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(RUN_SEGMENT_PATTERN).expect("run segment pattern"));

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProposalFile {
    proposals: Vec<Proposal>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Proposal {
    kind: MemoryKind,
    title: String,
    text: String,
    #[serde(default)]
    scope: MemoryScope,
    #[serde(default)]
    refs: Vec<MemoryRef>,
    #[serde(default)]
    confidence: Option<f64>,
}

/// Parses and validates one proposals document; nothing is created on error.
fn parse(content: &str) -> Result<Vec<NewMemoryDraft>> {
    let invalid =
        |error: serde_json::Error| MemoryError::Invalid(format!("invalid proposals file: {error}"));
    let mut document: Value = serde_json::from_str(content).map_err(invalid)?;
    canonicalize_document(&mut document);
    let file: ProposalFile = serde_json::from_value(document).map_err(invalid)?;
    if file.proposals.len() > MAX_PROPOSALS_PER_FILE {
        return Err(MemoryError::Invalid(format!(
            "{} proposals; at most {MAX_PROPOSALS_PER_FILE} per file",
            file.proposals.len()
        )));
    }
    Ok(file
        .proposals
        .into_iter()
        .map(|proposal| NewMemoryDraft {
            kind: proposal.kind,
            title: proposal.title,
            text: proposal.text,
            scope: proposal.scope,
            refs: proposal.refs,
            confidence: proposal.confidence.unwrap_or(DEFAULT_CONFIDENCE),
        })
        .collect())
}

struct NewMemoryDraft {
    kind: MemoryKind,
    title: String,
    text: String,
    scope: MemoryScope,
    refs: Vec<MemoryRef>,
    confidence: f64,
}

impl MemoryManager<'_> {
    /// Ingests a proposals file, or every `memory-proposals.json` under a folder.
    ///
    /// A refused file is listed in the report and creates nothing; the other
    /// files still go through.
    pub fn ingest(&self, project: &str, target: &Path) -> Result<IngestReport> {
        self.ingest_routed(project, target, &|_| None)
    }

    /// Like `ingest`; a proposal without route takes the route `route_of` gives its issue.
    pub fn ingest_routed(
        &self,
        project: &str,
        target: &Path,
        route_of: &dyn Fn(&str) -> Option<Route>,
    ) -> Result<IngestReport> {
        let files = proposal_files(target)?;
        let mut report = IngestReport::default();
        for file in files {
            report.files += 1;
            let outcome = aidp_core::text::read_text_lenient(&file)
                .map_err(MemoryError::from)
                .and_then(|content| parse(&content))
                .and_then(|drafts| {
                    let drafts = with_issue_route(with_path_refs(drafts, &file), route_of);
                    self.add_all(project, drafts)
                });
            match outcome {
                Ok((created, deduplicated, activated)) => {
                    report.created += created;
                    report.deduplicated += deduplicated;
                    report.activated += activated;
                }
                Err(error) => report.rejected.push(IngestRejection {
                    file: file.display().to_string(),
                    reason: error.to_string(),
                }),
            }
        }
        Ok(report)
    }

    fn add_all(&self, project: &str, drafts: Vec<NewMemoryDraft>) -> Result<(usize, usize, usize)> {
        for draft in &drafts {
            validate_text(&draft.title, &draft.text, draft.confidence)?;
        }
        let mut created = 0;
        let mut deduplicated = 0;
        let mut activated = 0;
        for draft in drafts {
            let outcome = self.add(NewMemory {
                project: project.to_string(),
                kind: draft.kind,
                title: draft.title,
                text: draft.text,
                scope: draft.scope,
                refs: draft.refs,
                confidence: draft.confidence,
                status: MemoryStatus::Proposed,
                supersedes: None,
                auto_activate: true,
            })?;
            if outcome.deduplicated {
                deduplicated += 1;
            } else {
                created += 1;
            }
            if outcome.activated {
                activated += 1;
            }
        }
        Ok((created, deduplicated, activated))
    }
}

/// Gives a proposal without route the route of its first issue.
fn with_issue_route(
    mut drafts: Vec<NewMemoryDraft>,
    route_of: &dyn Fn(&str) -> Option<Route>,
) -> Vec<NewMemoryDraft> {
    for draft in drafts
        .iter_mut()
        .filter(|draft| draft.scope.route.is_none())
    {
        draft.scope.route = draft
            .refs
            .iter()
            .filter(|reference| reference.ref_type == RefType::Issue)
            .find_map(|reference| route_of(&reference.id));
    }
    drafts
}

/// Adds the issue and run named by the vault path of the file, when absent.
fn with_path_refs(mut drafts: Vec<NewMemoryDraft>, file: &Path) -> Vec<NewMemoryDraft> {
    let found = refs_in_path(file);
    for draft in &mut drafts {
        for reference in &found {
            if !draft.refs.contains(reference) {
                draft.refs.push(reference.clone());
            }
        }
    }
    drafts
}

fn refs_in_path(file: &Path) -> Vec<MemoryRef> {
    file.components()
        .filter_map(|component| component.as_os_str().to_str())
        .filter_map(|segment| {
            let ref_type = if ISSUE_SEGMENT.is_match(segment) {
                RefType::Issue
            } else if RUN_SEGMENT.is_match(segment) {
                RefType::Run
            } else {
                return None;
            };
            Some(MemoryRef {
                ref_type,
                id: segment.to_string(),
            })
        })
        .collect()
}

fn proposal_files(target: &Path) -> Result<Vec<PathBuf>> {
    if target.is_file() {
        return Ok(vec![target.to_path_buf()]);
    }
    if !target.is_dir() {
        return Err(MemoryError::NotFound(format!(
            "proposals path not found: {}",
            target.display()
        )));
    }
    let mut files: Vec<PathBuf> = WalkDir::new(target)
        .max_depth(INGEST_MAX_DEPTH)
        .into_iter()
        .flatten()
        .filter(|entry| entry.file_type().is_file() && entry.file_name() == PROPOSALS_FILE)
        .map(|entry| entry.into_path())
        .collect();
    files.sort();
    Ok(files)
}

#[cfg(test)]
#[path = "../tests/unit/proposals_tests.rs"]
mod tests;
