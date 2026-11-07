//! Ingestion of `memory-proposals.json` files written by pipeline stages.

use crate::error::{MemoryError, Result};
use crate::manager::MemoryManager;
use crate::models::constants::limits::{
    DEFAULT_CONFIDENCE, INGEST_MAX_DEPTH, MAX_PROPOSALS_PER_FILE,
};
use crate::models::constants::paths::PROPOSALS_FILE;
use crate::models::enums::{MemoryKind, MemoryStatus};
use crate::models::ingest_report::{IngestRejection, IngestReport};
use crate::models::memory_ref::MemoryRef;
use crate::models::memory_scope::MemoryScope;
use crate::models::new_memory::NewMemory;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

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
    let file: ProposalFile = serde_json::from_str(content)
        .map_err(|error| MemoryError::Invalid(format!("invalid proposals file: {error}")))?;
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
        let files = proposal_files(target)?;
        let mut report = IngestReport::default();
        for file in files {
            report.files += 1;
            let outcome = std::fs::read_to_string(&file)
                .map_err(MemoryError::from)
                .and_then(|content| parse(&content))
                .and_then(|drafts| self.add_all(project, drafts));
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
