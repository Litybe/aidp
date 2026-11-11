//! File operations on the knowledge notes of a vault: write, rewrite with history, mark superseded, move.

use crate::error::{MemoryError, Result};
use crate::knowledge_note::parse_knowledge;
use crate::models::constants::knowledge::KNOWLEDGE_DIR;
use crate::models::constants::knowledge_proposals::{
    HISTORY_DIR, HISTORY_NAME_SEPARATOR, KEY_SOURCE, KEY_STATUS, KEY_SUPERSEDED_BY, KEY_UPDATED,
    NOTE_EXTENSION,
};
use crate::models::constants::paths::INBOX_DIR;
use crate::models::enums::NoteStatus;
use crate::note_paths::{absolute, free_path, knowledge_rel_path, wikilink};
use crate::note_text::{
    append_evidence, body_of, evidence_has_source, evidence_of, render_body, replace_body,
    set_field, without_evidence,
};
use std::path::{Path, PathBuf};

const NO_DATE: &str = "undated";

/// The notes of one vault, dated by a fixed `today`.
pub struct NoteVault {
    pub root: PathBuf,
    pub repo: String,
    pub today: String,
}

impl NoteVault {
    pub fn new(root: &Path, repo: &str, today: &str) -> Self {
        Self {
            root: root.to_path_buf(),
            repo: repo.to_string(),
            today: today.to_string(),
        }
    }

    /// The absolute path of a vault-relative path.
    pub fn path(&self, rel: &str) -> PathBuf {
        absolute(&self.root, rel)
    }

    /// `<vault>/Knowledge/_inbox`.
    pub fn inbox_dir(&self) -> PathBuf {
        self.root.join(KNOWLEDGE_DIR).join(INBOX_DIR)
    }

    pub fn exists(&self, rel: &str) -> bool {
        self.path(rel).is_file()
    }

    /// Reads a note; a missing one is a `NotFound`.
    pub fn read(&self, rel: &str) -> Result<String> {
        std::fs::read_to_string(self.path(rel))
            .map_err(|_| MemoryError::NotFound(format!("note not found: {rel}")))
    }

    /// Writes a note, creating its folders.
    pub fn write(&self, rel: &str, content: &str) -> Result<()> {
        self.write_at(&self.path(rel), content)
    }

    pub fn write_at(&self, path: &Path, content: &str) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, content)?;
        Ok(())
    }

    /// Whether a note already carries evidence from `source`.
    pub fn has_source(&self, rel: &str, source: &str) -> bool {
        self.read(rel)
            .is_ok_and(|content| evidence_has_source(&content, source))
    }

    /// Adds an evidence bullet to an existing note and bumps its `updated` date; a `source` already cited changes nothing.
    pub fn add_evidence(&self, rel: &str, source: &str, entry: &str) -> Result<()> {
        let content = self.read(rel)?;
        if evidence_has_source(&content, source) {
            return Ok(());
        }
        let content = set_field(&append_evidence(&content, entry), KEY_UPDATED, &self.today);
        self.write(rel, &content)
    }

    /// Replaces the body of a note, keeping its frontmatter and evidence; the previous file goes to `Knowledge/_history`.
    pub fn rewrite(
        &self,
        target: &str,
        title: &str,
        body: &str,
        source: &str,
        entry: &str,
    ) -> Result<()> {
        let target = knowledge_rel_path(target)?;
        let old = self.read(&target)?;
        let unchanged =
            without_evidence(&body_of(&old)).trim() == render_body(title, body, &[]).trim();
        if unchanged && evidence_has_source(&old, source) {
            return Ok(());
        }
        self.archive_version(&target, &old)?;
        let mut evidence = evidence_of(&old);
        evidence.push(entry.to_string());
        let fresh = replace_body(&old, &render_body(title, body, &evidence));
        let fresh = set_field(&fresh, KEY_UPDATED, &self.today);
        self.write(&target, &set_field(&fresh, KEY_SOURCE, source))
    }

    fn archive_version(&self, rel: &str, content: &str) -> Result<()> {
        let flattened = rel
            .strip_prefix(&format!("{KNOWLEDGE_DIR}/"))
            .unwrap_or(rel)
            .trim_end_matches(&format!(".{NOTE_EXTENSION}"))
            .replace('/', HISTORY_NAME_SEPARATOR);
        let parsed = parse_knowledge(content, NO_DATE);
        let stamp: String = parsed
            .front
            .updated
            .as_deref()
            .unwrap_or(NO_DATE)
            .chars()
            .filter(|character| character.is_ascii_alphanumeric() || *character == '-')
            .collect();
        let dir = self.root.join(KNOWLEDGE_DIR).join(HISTORY_DIR);
        self.write_at(
            &free_path(&dir, &format!("{flattened}{HISTORY_NAME_SEPARATOR}{stamp}")),
            content,
        )
    }

    /// Marks a note superseded by another and bumps its `updated` date.
    pub fn mark_superseded(&self, target: &str, replacement: &str) -> Result<()> {
        let target = knowledge_rel_path(target)?;
        let content = self.read(&target)?;
        let content = set_field(&content, KEY_STATUS, NoteStatus::Superseded.as_str());
        let content = set_field(&content, KEY_SUPERSEDED_BY, &wikilink(replacement));
        self.write(&target, &set_field(&content, KEY_UPDATED, &self.today))
    }

    /// Moves a file into `dir` under a free name; returns its new path.
    pub fn move_to(&self, file: &Path, dir: &Path) -> Result<PathBuf> {
        let stem = file
            .file_stem()
            .map(|stem| stem.to_string_lossy().to_string())
            .unwrap_or_default();
        std::fs::create_dir_all(dir)?;
        let target = free_path(dir, &stem);
        std::fs::rename(file, &target)?;
        Ok(target)
    }
}
