//! Turns `knowledge-proposals.json` into notes of the vault: adds, merges, inbox entries, rewrites, replacements.

use crate::embedder::Embedder;
use crate::embedding_index::embedding_text;
use crate::error::{MemoryError, Result};
use crate::knowledge_note::parse_knowledge;
use crate::knowledge_proposals::parse_knowledge_proposals;
use crate::models::constants::knowledge_proposals::{
    DATE_FORMAT, FALLBACK_CANDIDATES, FALLBACK_JACCARD, INBOX_COSINE, INBOX_ONLY_DOMAINS,
    KEY_ACTION, KEY_CANDIDATE_OF, KEY_REPO, KEY_SCOPE, KEY_SOURCE, KEY_TARGET, MERGE_COSINE,
    NAME_COLLISION_SEPARATOR,
};
use crate::models::constants::schema::GLOBAL_PROJECT;
use crate::models::duplicate_verdict::DuplicateVerdict;
use crate::models::enums::{
    Domain, NoteStatus, ProposalAction, ProposalOutcome, ProposalScope, Source,
};
use crate::models::ingest_report::IngestRejection;
use crate::models::knowledge_ingest_report::KnowledgeIngestReport;
use crate::models::knowledge_proposal::KnowledgeProposal;
use crate::models::note_draft::NoteDraft;
use crate::models::proposal_result::ProposalResult;
use crate::models::search_query::SearchQuery;
use crate::note_paths::{
    ensure_note_target, knowledge_rel_path, note_rel_path, vault_relative, wikilink,
};
use crate::note_text::{body_of, render_note};
use crate::note_vault::NoteVault;
use crate::retriever::{Bm25Retriever, Retriever};
use crate::store::MemoryStore;
use crate::text::{jaccard, word_set};
use crate::vectors::{cosine, decode_vector};
use rusqlite::params;
use std::collections::HashSet;
use std::path::Path;

const HEAD_CHARS: usize = 1200;
const QUERY_BODY_CHARS: usize = 200;
const SEPARATOR: &str = ": ";
const WORD_SEPARATOR: &str = " ";

/// Reconciles knowledge proposals with the notes already in the vault.
pub struct KnowledgeIngestor<'a> {
    store: &'a MemoryStore,
    vault: NoteVault,
    project: String,
    embedder: Option<&'a dyn Embedder>,
}

impl<'a> KnowledgeIngestor<'a> {
    /// `repo` is the game folder under `Knowledge/`, `project` the key of its index.
    pub fn new(store: &'a MemoryStore, vault_root: &Path, repo: &str, project: &str) -> Self {
        let today = chrono::Utc::now().format(DATE_FORMAT).to_string();
        Self {
            store,
            vault: NoteVault::new(vault_root, repo, &today),
            project: project.to_string(),
            embedder: None,
        }
    }

    /// Compares proposals with the vectors of the existing notes instead of their words.
    pub fn with_embedder(mut self, embedder: &'a dyn Embedder) -> Self {
        self.embedder = Some(embedder);
        self
    }

    /// Fixes the date written in `updated` and in evidence lines.
    pub fn with_today(mut self, today: &str) -> Self {
        self.vault.today = today.to_string();
        self
    }

    /// Ingests one proposals file; `source` is the `source` of the notes, such as `run:<id>`.
    pub fn ingest_file(&self, file: &Path, source: &str) -> Result<KnowledgeIngestReport> {
        let bytes = std::fs::read(file)?;
        self.ingest_bytes(&file.display().to_string(), &bytes, source)
    }

    /// Ingests the content of a proposals file; an invalid file is reported and creates nothing.
    pub fn ingest_bytes(
        &self,
        label: &str,
        bytes: &[u8],
        source: &str,
    ) -> Result<KnowledgeIngestReport> {
        let mut report = KnowledgeIngestReport::default();
        let file = match parse_knowledge_proposals(bytes) {
            Ok(file) => file,
            Err(error) => {
                report.rejected.push(IngestRejection {
                    file: label.to_string(),
                    reason: error.to_string(),
                });
                return Ok(report);
            }
        };
        report.files = 1;
        for proposal in &file.proposals {
            match self.apply(proposal, source) {
                Ok(result) => record(&mut report, result),
                Err(error) => report.rejected.push(IngestRejection {
                    file: format!("{label}#{}", proposal.slug),
                    reason: error.to_string(),
                }),
            }
        }
        Ok(report)
    }

    fn apply(&self, proposal: &KnowledgeProposal, source: &str) -> Result<ProposalResult> {
        match proposal.action {
            ProposalAction::Add => self.add(proposal, source),
            ProposalAction::Update => self.update(proposal, source),
            ProposalAction::Supersede => self.supersede(proposal, source),
        }
    }

    fn add(&self, proposal: &KnowledgeProposal, source: &str) -> Result<ProposalResult> {
        let verdict = self.verdict(proposal)?;
        let destination = self.destination(proposal);
        if self.vault.has_source(&destination, source) {
            return Ok(result(proposal, ProposalOutcome::Merged, &destination));
        }
        if INBOX_ONLY_DOMAINS.contains(&proposal.domain) {
            return self.to_inbox(proposal, source, candidate_of(&verdict));
        }
        match verdict {
            DuplicateVerdict::Same(existing) if self.vault.exists(&existing) => {
                self.vault.add_evidence(
                    &existing,
                    source,
                    &self.evidence_entry(proposal, source),
                )?;
                Ok(result(proposal, ProposalOutcome::Merged, &existing))
            }
            DuplicateVerdict::Close(existing) if self.vault.exists(&existing) => {
                self.to_inbox(proposal, source, Some(existing))
            }
            _ if self.vault.exists(&destination) => {
                self.to_inbox(proposal, source, Some(destination))
            }
            _ => {
                self.write_new(proposal, source, &destination, Vec::new())?;
                Ok(result(proposal, ProposalOutcome::Added, &destination))
            }
        }
    }

    fn update(&self, proposal: &KnowledgeProposal, source: &str) -> Result<ProposalResult> {
        let target = self.target_of(proposal)?;
        if self.is_inbox_only(proposal, &target)? {
            return self.to_inbox(proposal, source, Some(target));
        }
        self.vault.rewrite(
            &target,
            proposal.title.trim(),
            &proposal.body,
            source,
            &self.evidence_entry(proposal, source),
        )?;
        Ok(result(proposal, ProposalOutcome::Updated, &target))
    }

    fn supersede(&self, proposal: &KnowledgeProposal, source: &str) -> Result<ProposalResult> {
        let target = self.target_of(proposal)?;
        if self.is_inbox_only(proposal, &target)? {
            return self.to_inbox(proposal, source, Some(target));
        }
        let destination = self.destination(proposal);
        if self.vault.has_source(&destination, source) {
            return Ok(result(proposal, ProposalOutcome::Superseded, &destination));
        }
        if self.vault.exists(&destination) {
            return self.to_inbox(proposal, source, Some(destination));
        }
        self.write_new(proposal, source, &destination, Vec::new())?;
        self.vault.mark_superseded(&target, &destination)?;
        Ok(result(proposal, ProposalOutcome::Superseded, &destination))
    }

    fn target_of(&self, proposal: &KnowledgeProposal) -> Result<String> {
        let target = knowledge_rel_path(proposal.target.as_deref().unwrap_or_default())?;
        ensure_note_target(&target, &self.vault.repo)?;
        if !self.vault.exists(&target) {
            return Err(MemoryError::NotFound(format!(
                "target note not found: {target}"
            )));
        }
        Ok(target)
    }

    fn is_inbox_only(&self, proposal: &KnowledgeProposal, target: &str) -> Result<bool> {
        if INBOX_ONLY_DOMAINS.contains(&proposal.domain) {
            return Ok(true);
        }
        let content = self.vault.read(target)?;
        let target_domain = parse_knowledge(&content, target)
            .front
            .domain
            .and_then(|domain| Domain::parse(&domain).ok());
        Ok(target_domain.is_some_and(|domain| INBOX_ONLY_DOMAINS.contains(&domain)))
    }

    fn destination(&self, proposal: &KnowledgeProposal) -> String {
        note_rel_path(
            proposal.scope,
            &self.vault.repo,
            proposal.domain,
            &proposal.slug,
        )
    }

    fn evidence_entry(&self, proposal: &KnowledgeProposal, source: &str) -> String {
        let evidence = proposal
            .evidence
            .as_deref()
            .map(|text| {
                text.split_whitespace()
                    .collect::<Vec<_>>()
                    .join(WORD_SEPARATOR)
            })
            .filter(|text| !text.is_empty());
        match evidence {
            Some(text) => format!("{} {source}{SEPARATOR}{text}", self.vault.today),
            None => format!("{} {source}", self.vault.today),
        }
    }

    fn draft(
        &self,
        proposal: &KnowledgeProposal,
        source: &str,
        status: NoteStatus,
        extra: Vec<(String, String)>,
    ) -> NoteDraft {
        NoteDraft {
            domain: proposal.domain,
            tags: proposal.tags.clone(),
            agents: proposal.agents.clone(),
            status,
            importance: proposal.importance,
            updated: self.vault.today.clone(),
            source: source.to_string(),
            title: proposal.title.trim().to_string(),
            body: proposal.body.clone(),
            extra,
            evidence: vec![self.evidence_entry(proposal, source)],
        }
    }

    fn write_new(
        &self,
        proposal: &KnowledgeProposal,
        source: &str,
        destination: &str,
        extra: Vec<(String, String)>,
    ) -> Result<()> {
        let draft = self.draft(proposal, source, NoteStatus::Active, extra);
        self.vault.write(destination, &render_note(&draft))
    }

    fn to_inbox(
        &self,
        proposal: &KnowledgeProposal,
        source: &str,
        candidate: Option<String>,
    ) -> Result<ProposalResult> {
        if let Some(existing) = self.inboxed_from(&proposal.slug, source) {
            return Ok(result(proposal, ProposalOutcome::Inboxed, &existing));
        }
        let mut extra = vec![
            (KEY_ACTION.to_string(), proposal.action.to_string()),
            (KEY_SCOPE.to_string(), proposal.scope.to_string()),
        ];
        if proposal.scope == ProposalScope::Project {
            extra.push((KEY_REPO.to_string(), self.vault.repo.clone()));
        }
        if let Some(target) = proposal.target.as_deref() {
            extra.push((KEY_TARGET.to_string(), wikilink_or_raw(target)));
        }
        if let Some(existing) = candidate {
            extra.push((KEY_CANDIDATE_OF.to_string(), wikilink(&existing)));
        }
        let draft = self.draft(proposal, source, NoteStatus::Proposed, extra);
        let path = crate::note_paths::free_path(&self.vault.inbox_dir(), &proposal.slug);
        self.vault.write_at(&path, &render_note(&draft))?;
        Ok(result(
            proposal,
            ProposalOutcome::Inboxed,
            &vault_relative(&self.vault.root, &path),
        ))
    }

    fn inboxed_from(&self, slug: &str, source: &str) -> Option<String> {
        let marker = format!("{KEY_SOURCE}: {source}");
        let collision = format!("{slug}{NAME_COLLISION_SEPARATOR}");
        std::fs::read_dir(self.vault.inbox_dir())
            .ok()?
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                let stem = path
                    .file_stem()
                    .map(|stem| stem.to_string_lossy().to_string())
                    .unwrap_or_default();
                path.is_file()
                    && (stem == slug
                        || stem.strip_prefix(&collision).is_some_and(|suffix| {
                            !suffix.is_empty() && suffix.chars().all(|digit| digit.is_ascii_digit())
                        }))
            })
            .find(|path| {
                std::fs::read_to_string(path)
                    .is_ok_and(|content| content.lines().any(|line| line.trim() == marker))
            })
            .map(|path| vault_relative(&self.vault.root, &path))
    }

    fn verdict(&self, proposal: &KnowledgeProposal) -> Result<DuplicateVerdict> {
        if let Some(embedder) = self.embedder {
            if let Some(verdict) = self.vector_verdict(embedder, proposal)? {
                return Ok(verdict);
            }
        }
        self.overlap_verdict(proposal)
    }

    fn vector_verdict(
        &self,
        embedder: &dyn Embedder,
        proposal: &KnowledgeProposal,
    ) -> Result<Option<DuplicateVerdict>> {
        let head: String = proposal.body.chars().take(HEAD_CHARS).collect();
        let whole = embedding_text(&proposal.title, &proposal.body);
        let short = embedding_text(&proposal.title, &head);
        let queries = embedder.embed(&[whole.as_str(), short.as_str()])?;
        let rows = self.stored_vectors(embedder.model_id())?;
        let mut best: Option<(String, f64)> = None;
        for (path, vector) in rows {
            if !self.vault.exists(&path) {
                continue;
            }
            let score = queries
                .iter()
                .map(|query| cosine(query, &vector))
                .fold(0.0_f64, f64::max);
            if best.as_ref().is_none_or(|(_, top)| score > *top) {
                best = Some((path, score));
            }
        }
        Ok(best.map(|(path, score)| {
            if score >= MERGE_COSINE {
                DuplicateVerdict::Same(path)
            } else if score >= INBOX_COSINE {
                DuplicateVerdict::Close(path)
            } else {
                DuplicateVerdict::Fresh
            }
        }))
    }

    fn stored_vectors(&self, model: &str) -> Result<Vec<(String, Vec<f32>)>> {
        self.store.read(|connection| {
            let mut statement = connection.prepare(
                "SELECT c.path, v.vector FROM chunk_vectors v JOIN chunks c ON c.rowid = v.chunk_id \
                 WHERE c.source = ?1 AND c.project IN (?2, ?3) AND v.model = ?4",
            )?;
            let mut rows = statement.query(params![
                Source::Knowledge.as_str(),
                self.project,
                GLOBAL_PROJECT,
                model
            ])?;
            let mut vectors = Vec::new();
            while let Some(row) = rows.next()? {
                let path: String = row.get(0)?;
                let blob: Vec<u8> = row.get(1)?;
                vectors.push((path, decode_vector(&blob)?));
            }
            Ok(vectors)
        })
    }

    fn overlap_verdict(&self, proposal: &KnowledgeProposal) -> Result<DuplicateVerdict> {
        let lead: String = proposal.body.chars().take(QUERY_BODY_CHARS).collect();
        let mut query = SearchQuery::new(
            self.project.clone(),
            format!("{}{WORD_SEPARATOR}{lead}", proposal.title),
        );
        query.sources = vec![Source::Knowledge];
        query.k = FALLBACK_CANDIDATES;
        let hits = Bm25Retriever::new(self.store, None).search(&query)?;
        let proposed = word_set(&format!(
            "{}{WORD_SEPARATOR}{}",
            proposal.title, proposal.body
        ));
        let mut seen: HashSet<String> = HashSet::new();
        let mut best: Option<(String, f64)> = None;
        for hit in hits {
            if !seen.insert(hit.path.clone()) {
                continue;
            }
            let Ok(content) = self.vault.read(&hit.path) else {
                continue;
            };
            let existing = parse_knowledge(&content, &hit.path);
            let words = word_set(&format!(
                "{}{WORD_SEPARATOR}{}",
                existing.title,
                body_of(&content)
            ));
            let score = jaccard(&proposed, &words);
            if best.as_ref().is_none_or(|(_, top)| score > *top) {
                best = Some((hit.path, score));
            }
        }
        Ok(match best {
            Some((path, score)) if score >= FALLBACK_JACCARD => DuplicateVerdict::Same(path),
            _ => DuplicateVerdict::Fresh,
        })
    }
}

fn candidate_of(verdict: &DuplicateVerdict) -> Option<String> {
    match verdict {
        DuplicateVerdict::Same(path) | DuplicateVerdict::Close(path) => Some(path.clone()),
        DuplicateVerdict::Fresh => None,
    }
}

fn wikilink_or_raw(target: &str) -> String {
    knowledge_rel_path(target).map_or_else(
        |_| serde_json::to_string(target).unwrap_or_default(),
        |rel| wikilink(&rel),
    )
}

fn result(proposal: &KnowledgeProposal, outcome: ProposalOutcome, path: &str) -> ProposalResult {
    ProposalResult {
        slug: proposal.slug.clone(),
        outcome,
        path: path.to_string(),
    }
}

fn record(report: &mut KnowledgeIngestReport, result: ProposalResult) {
    match result.outcome {
        ProposalOutcome::Added => report.added += 1,
        ProposalOutcome::Merged => report.merged += 1,
        ProposalOutcome::Inboxed => report.inboxed += 1,
        ProposalOutcome::Updated => report.updated += 1,
        ProposalOutcome::Superseded => report.superseded += 1,
    }
    report.results.push(result);
}

#[cfg(test)]
#[path = "../tests/unit/knowledge_ingest_tests.rs"]
mod tests;
