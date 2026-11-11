//! Review of the proposed knowledge notes waiting in `<vault>/Knowledge/_inbox`.

use crate::error::{MemoryError, Result};
use crate::knowledge_note::{parse_knowledge, split_frontmatter};
use crate::models::constants::knowledge_proposals::{
    APPLIED_DIR, DATE_FORMAT, FOLDER_SEPARATOR, INBOX_KEYS, KEY_STATUS, KEY_UPDATED,
    NOTE_EXTENSION, REJECTED_DIR, WIKILINK_CLOSE, WIKILINK_OPEN,
};
use crate::models::enums::{Domain, NoteStatus, ProposalAction, ProposalOutcome, ProposalScope};
use crate::models::inbox_decision::InboxDecision;
use crate::models::inbox_entry::InboxEntry;
use crate::models::inbox_frontmatter::InboxFrontmatter;
use crate::note_paths::{knowledge_rel_path, note_rel_path, vault_relative};
use crate::note_text::{body_of, remove_fields, set_field, without_evidence};
use crate::note_vault::NoteVault;
use std::path::{Path, PathBuf};

const QUOTE: char = '"';

/// Lists, approves and rejects the notes of the inbox.
pub struct KnowledgeInbox {
    vault: NoteVault,
}

impl KnowledgeInbox {
    /// `repo` is the game folder used for a project-scoped note that does not name its own.
    pub fn new(vault_root: &Path, repo: &str) -> Self {
        let today = chrono::Utc::now().format(DATE_FORMAT).to_string();
        Self {
            vault: NoteVault::new(vault_root, repo, &today),
        }
    }

    /// Fixes the date written in `updated`.
    pub fn with_today(mut self, today: &str) -> Self {
        self.vault.today = today.to_string();
        self
    }

    /// The notes waiting for review, by file name.
    pub fn list(&self) -> Result<Vec<InboxEntry>> {
        let dir = self.vault.inbox_dir();
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return Ok(Vec::new());
        };
        let mut files: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| is_note(path))
            .collect();
        files.sort();
        files
            .iter()
            .map(|path| self.entry_of(path))
            .collect::<Result<Vec<_>>>()
    }

    fn entry_of(&self, path: &Path) -> Result<InboxEntry> {
        let content = std::fs::read_to_string(path)?;
        let stem = stem_of(path);
        let front = front_of(&content);
        Ok(InboxEntry {
            path: vault_relative(&self.vault.root, path),
            title: parse_knowledge(&content, &stem).title,
            domain: front.domain.clone().unwrap_or_default(),
            action: front
                .action
                .clone()
                .unwrap_or_else(|| ProposalAction::Add.to_string()),
            target: front.target.as_deref().map(unlink),
            candidate_of: front.candidate_of.as_deref().map(unlink),
            source: front.source.clone(),
        })
    }

    /// Applies an inbox note: an add or supersede moves it to its folder as `active`, an update rewrites its target.
    pub fn approve(&self, name: &str) -> Result<InboxDecision> {
        let file = self.resolve(name)?;
        let content = std::fs::read_to_string(&file)?;
        let front = front_of(&content);
        let action = front
            .action
            .as_deref()
            .map(ProposalAction::parse)
            .transpose()?
            .unwrap_or(ProposalAction::Add);
        match action {
            ProposalAction::Update => self.approve_update(&file, &content, &front),
            ProposalAction::Add | ProposalAction::Supersede => {
                self.approve_new(&file, &content, &front, action)
            }
        }
    }

    fn approve_new(
        &self,
        file: &Path,
        content: &str,
        front: &InboxFrontmatter,
        action: ProposalAction,
    ) -> Result<InboxDecision> {
        let domain = Domain::parse(front.domain.as_deref().unwrap_or_default())?;
        let scope = front
            .scope
            .as_deref()
            .map(ProposalScope::parse)
            .transpose()?
            .unwrap_or(ProposalScope::Project);
        let repo = front.repo.as_deref().unwrap_or(&self.vault.repo);
        let destination = note_rel_path(scope, repo, domain, &stem_of(file));
        if self.vault.exists(&destination) {
            return Err(MemoryError::Conflict(format!(
                "a note already exists at {destination}"
            )));
        }
        let active = remove_fields(content, &INBOX_KEYS);
        let active = set_field(&active, KEY_STATUS, NoteStatus::Active.as_str());
        let active = set_field(&active, KEY_UPDATED, &self.vault.today);
        let superseded = (action == ProposalAction::Supersede)
            .then(|| front.target.as_deref().map(unlink))
            .flatten();
        self.vault.write(&destination, &active)?;
        if let Some(target) = superseded {
            self.vault
                .mark_superseded(&knowledge_rel_path(&target)?, &destination)?;
        }
        std::fs::remove_file(file)?;
        let outcome = if action == ProposalAction::Supersede {
            ProposalOutcome::Superseded
        } else {
            ProposalOutcome::Added
        };
        Ok(InboxDecision {
            outcome: Some(outcome),
            path: destination,
        })
    }

    fn approve_update(
        &self,
        file: &Path,
        content: &str,
        front: &InboxFrontmatter,
    ) -> Result<InboxDecision> {
        let target =
            knowledge_rel_path(&unlink(front.target.as_deref().ok_or_else(|| {
                MemoryError::Invalid("an inbox update needs a target".to_string())
            })?))?;
        let title = parse_knowledge(content, &stem_of(file)).title;
        let source = front.source.clone().unwrap_or_default();
        let entry = format!("{} {source} (approved)", self.vault.today);
        let body = without_evidence(&body_of(content));
        self.vault
            .rewrite(&target, &title, &body, &source, &entry)?;
        self.vault
            .move_to(file, &self.vault.inbox_dir().join(APPLIED_DIR))?;
        Ok(InboxDecision {
            outcome: Some(ProposalOutcome::Updated),
            path: target,
        })
    }

    /// Moves an inbox note to `_inbox/_rejected/`; nothing is deleted.
    pub fn reject(&self, name: &str) -> Result<InboxDecision> {
        let file = self.resolve(name)?;
        let moved = self
            .vault
            .move_to(&file, &self.vault.inbox_dir().join(REJECTED_DIR))?;
        Ok(InboxDecision {
            outcome: None,
            path: vault_relative(&self.vault.root, &moved),
        })
    }

    fn resolve(&self, name: &str) -> Result<PathBuf> {
        let unified = name.replace('\\', &FOLDER_SEPARATOR.to_string());
        let file_name = unified.rsplit(FOLDER_SEPARATOR).next().unwrap_or_default();
        if file_name.is_empty() || file_name == ".." {
            return Err(MemoryError::Invalid(format!("invalid inbox note `{name}`")));
        }
        let with_extension = if file_name.ends_with(&format!(".{NOTE_EXTENSION}")) {
            file_name.to_string()
        } else {
            format!("{file_name}.{NOTE_EXTENSION}")
        };
        let path = self.vault.inbox_dir().join(with_extension);
        if !path.is_file() {
            return Err(MemoryError::NotFound(format!(
                "inbox note not found: {name}"
            )));
        }
        Ok(path)
    }
}

fn is_note(path: &Path) -> bool {
    path.is_file()
        && path
            .extension()
            .is_some_and(|extension| extension == NOTE_EXTENSION)
}

fn stem_of(path: &Path) -> String {
    path.file_stem()
        .map(|stem| stem.to_string_lossy().to_string())
        .unwrap_or_default()
}

fn front_of(content: &str) -> InboxFrontmatter {
    split_frontmatter(content)
        .and_then(|(yaml, _)| serde_yaml::from_str(&yaml).ok())
        .unwrap_or_default()
}

/// `[[Knowledge/a/b]]` becomes `Knowledge/a/b.md`; other values are returned trimmed.
fn unlink(value: &str) -> String {
    let trimmed = value.trim().trim_matches(QUOTE);
    match trimmed
        .strip_prefix(WIKILINK_OPEN)
        .and_then(|rest| rest.strip_suffix(WIKILINK_CLOSE))
    {
        Some(stem) if !stem.ends_with(&format!(".{NOTE_EXTENSION}")) => {
            format!("{stem}.{NOTE_EXTENSION}")
        }
        Some(stem) => stem.to_string(),
        None => trimmed.to_string(),
    }
}

#[cfg(test)]
#[path = "../tests/unit/knowledge_inbox_tests.rs"]
mod tests;
