//! Reads and validates `knowledge-proposals.json`.

use crate::error::{MemoryError, Result};
use crate::models::constants::knowledge_proposals::{
    MAX_EVIDENCE_CHARS, MAX_KNOWLEDGE_PROPOSALS, MAX_NOTE_BODY_CHARS, MAX_NOTE_TITLE_CHARS,
};
use crate::models::enums::ProposalAction;
use crate::models::knowledge_proposal::KnowledgeProposal;
use crate::models::knowledge_proposal_file::KnowledgeProposalFile;
use crate::note_paths::knowledge_rel_path;
use crate::note_text::has_evidence_heading;
use regex::Regex;
use std::sync::OnceLock;

const SLUG_PATTERN: &str = r"^[a-z0-9]+(?:-[a-z0-9]+)*$";
const LABEL_PATTERN: &str = r"^[A-Za-z0-9][A-Za-z0-9._:/ -]*$";
const MAX_SLUG_CHARS: usize = 80;
const MAX_LABEL_CHARS: usize = 60;
const MAX_LABELS: usize = 20;

fn slug_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| Regex::new(SLUG_PATTERN).expect("the slug pattern is valid"))
}

fn label_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| Regex::new(LABEL_PATTERN).expect("the label pattern is valid"))
}

fn invalid(slug: &str, reason: impl std::fmt::Display) -> MemoryError {
    MemoryError::Invalid(format!("proposal `{slug}`: {reason}"))
}

fn check_labels(slug: &str, name: &str, labels: &[String]) -> Result<()> {
    if labels.len() > MAX_LABELS {
        return Err(invalid(slug, format!("more than {MAX_LABELS} {name}")));
    }
    for label in labels {
        if label.chars().count() > MAX_LABEL_CHARS || !label_pattern().is_match(label) {
            return Err(invalid(slug, format!("invalid {name} entry `{label}`")));
        }
    }
    Ok(())
}

/// Checks one proposal against the limits of the format.
pub fn validate_proposal(proposal: &KnowledgeProposal) -> Result<()> {
    let slug = proposal.slug.as_str();
    if slug.len() > MAX_SLUG_CHARS || !slug_pattern().is_match(slug) {
        return Err(invalid(slug, "slug must be kebab-case (a-z, 0-9, dashes)"));
    }
    let title = proposal.title.trim();
    if title.is_empty() || title.chars().count() > MAX_NOTE_TITLE_CHARS || title.contains('\n') {
        return Err(invalid(
            slug,
            format!("title must be one line of 1 to {MAX_NOTE_TITLE_CHARS} characters"),
        ));
    }
    let body_length = proposal.body.trim().chars().count();
    if body_length == 0 || body_length > MAX_NOTE_BODY_CHARS {
        return Err(invalid(
            slug,
            format!("body must hold 1 to {MAX_NOTE_BODY_CHARS} characters"),
        ));
    }
    if !(0.0..=1.0).contains(&proposal.importance) {
        return Err(invalid(slug, "importance must be between 0 and 1"));
    }
    check_labels(slug, "tags", &proposal.tags)?;
    check_labels(slug, "agents", &proposal.agents)?;
    if proposal
        .evidence
        .as_deref()
        .is_some_and(|text| text.chars().count() > MAX_EVIDENCE_CHARS)
    {
        return Err(invalid(
            slug,
            format!("evidence is over {MAX_EVIDENCE_CHARS} characters"),
        ));
    }
    if has_evidence_heading(&proposal.body) {
        return Err(invalid(slug, "body must not hold an evidence heading"));
    }
    if let Some(target) = proposal
        .target
        .as_deref()
        .filter(|target| !target.is_empty())
    {
        if target
            .chars()
            .any(|character| character.is_control() || character == '"')
        {
            return Err(invalid(slug, "target holds a control character or a quote"));
        }
        knowledge_rel_path(target).map_err(|error| invalid(slug, error))?;
    }
    let needs_target = matches!(
        proposal.action,
        ProposalAction::Update | ProposalAction::Supersede
    );
    if needs_target && proposal.target.as_deref().is_none_or(str::is_empty) {
        return Err(invalid(
            slug,
            format!("action `{}` needs a target", proposal.action),
        ));
    }
    Ok(())
}

/// Parses a `knowledge-proposals.json`; one invalid proposal, an unknown field or more than five proposals refuse the whole file.
pub fn parse_knowledge_proposals(bytes: &[u8]) -> Result<KnowledgeProposalFile> {
    let file: KnowledgeProposalFile = serde_json::from_slice(bytes)
        .map_err(|error| MemoryError::Invalid(format!("invalid knowledge proposals: {error}")))?;
    if file.proposals.len() > MAX_KNOWLEDGE_PROPOSALS {
        return Err(MemoryError::Invalid(format!(
            "{} proposals; at most {MAX_KNOWLEDGE_PROPOSALS} per file",
            file.proposals.len()
        )));
    }
    for proposal in &file.proposals {
        validate_proposal(proposal)?;
    }
    Ok(file)
}

#[cfg(test)]
#[path = "../tests/unit/knowledge_proposals_tests.rs"]
mod tests;
