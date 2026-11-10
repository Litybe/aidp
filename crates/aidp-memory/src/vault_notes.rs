//! Two-way synchronisation between the memories and their Markdown notes in the vault.
//!
//! The notes are the source of truth for what a human edits; the database stays rebuildable.

use crate::error::{MemoryError, Result};
use crate::manager::{
    format_id, load, merge_refs, parse_id, row_to_memory, save, supersede_previous, sync_chunks,
    validate_text, MemoryManager, MEMORY_COLUMNS,
};
use crate::models::constants::limits::DEFAULT_CONFIDENCE;
use crate::models::constants::notes::{
    ARCHIVE_DIR, FRONTMATTER_FENCE, NOTE_DEFAULT_KIND, NOTE_FILE_ID_PATTERN, NOTE_REF_SEPARATOR,
    NO_ROUTE_DIR, SLUG_MAX_CHARS, SLUG_SEPARATOR, TITLE_PREFIX,
};
use crate::models::constants::paths::MARKDOWN_EXT;
use crate::models::enums::{MemoryStatus, RefType};
use crate::models::ingest_report::IngestRejection;
use crate::models::memory::Memory;
use crate::models::memory_note::MemoryNote;
use crate::models::memory_ref::MemoryRef;
use crate::models::memory_scope::MemoryScope;
use crate::models::vault_sync_report::VaultSyncReport;
use crate::text::{fold, short_hash};
use regex::Regex;
use rusqlite::{params, Connection};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use walkdir::WalkDir;

static NOTE_FILE_ID: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(NOTE_FILE_ID_PATTERN).expect("note file id pattern"));

/// A note read from the vault.
struct ParsedNote {
    front: MemoryNote,
    title: String,
    text: String,
}

/// What every note of one synchronisation is applied against.
struct Pass<'a> {
    project: &'a str,
    now: &'a str,
    hashes: &'a HashMap<i64, Option<String>>,
}

/// What one synchronisation tracks while it walks the notes.
#[derive(Default)]
struct Walk {
    seen: HashSet<i64>,
    locations: HashMap<i64, PathBuf>,
    untouchable: HashSet<i64>,
    report: VaultSyncReport,
}

impl MemoryManager<'_> {
    /// Applies the edited notes under `notes_dir`, then writes every note that differs.
    pub fn sync_vault(&self, project: &str, notes_dir: &Path) -> Result<VaultSyncReport> {
        let now = self.now();
        self.store.write(|connection| {
            let hashes = stored_hashes(connection, project)?;
            let files = note_files(notes_dir);
            let mut walk = Walk::default();
            let pass = Pass {
                project,
                now: &now,
                hashes: &hashes,
            };
            for file in &files {
                apply_file(connection, &pass, file, &mut walk);
            }
            if !files.is_empty() {
                archive_deleted(connection, project, &hashes, &now, &mut walk)?;
            }
            export(connection, project, notes_dir, &mut walk)?;
            let report = walk.report;
            if report.imported + report.created + report.archived > 0 {
                sync_chunks(connection, project)?;
            }
            Ok(report)
        })
    }
}

fn stored_hashes(connection: &Connection, project: &str) -> Result<HashMap<i64, Option<String>>> {
    let mut statement =
        connection.prepare("SELECT num, note_hash FROM memories WHERE project = ?1")?;
    let rows = statement.query_map(params![project], |row| Ok((row.get(0)?, row.get(1)?)))?;
    Ok(rows.collect::<std::result::Result<HashMap<_, _>, _>>()?)
}

fn note_files(notes_dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = WalkDir::new(notes_dir)
        .into_iter()
        .filter_entry(|entry| {
            entry.depth() == 0 || !entry.file_name().to_string_lossy().starts_with('.')
        })
        .flatten()
        .filter(|entry| {
            entry.file_type().is_file()
                && entry
                    .path()
                    .extension()
                    .is_some_and(|extension| extension == MARKDOWN_EXT)
        })
        .map(walkdir::DirEntry::into_path)
        .collect();
    files.sort();
    files
}

fn number_from_file_name(file: &Path) -> Option<i64> {
    let stem = file.file_stem()?.to_str()?;
    NOTE_FILE_ID.captures(stem)?.get(1)?.as_str().parse().ok()
}

fn apply_file(connection: &Connection, pass: &Pass<'_>, file: &Path, walk: &mut Walk) {
    let outcome = aidp_core::text::read_text_lenient(file)
        .map_err(MemoryError::from)
        .and_then(|content| {
            let note = parse_note(&content)?;
            apply_note(connection, pass, file, &content, note, walk)
        });
    if let Err(error) = outcome {
        if let Some(number) = number_from_file_name(file) {
            walk.seen.insert(number);
            walk.untouchable.insert(number);
        }
        walk.report.rejected.push(IngestRejection {
            file: file.display().to_string(),
            reason: error.to_string(),
        });
    }
}

fn apply_note(
    connection: &Connection,
    pass: &Pass<'_>,
    file: &Path,
    content: &str,
    note: ParsedNote,
    walk: &mut Walk,
) -> Result<()> {
    let (project, now) = (pass.project, pass.now);
    let confidence = note.front.confidence.unwrap_or(DEFAULT_CONFIDENCE);
    validate_text(&note.title, &note.text, confidence)?;
    let refs = parse_refs(&note.front.refs)?;
    let number = match note.front.id.as_deref().map(parse_id).transpose()? {
        Some(number) => {
            walk.seen.insert(number);
            match pass.hashes.get(&number) {
                Some(stored) => {
                    if stored.as_deref() != Some(short_hash(content).as_str()) {
                        apply_edit(connection, project, number, &note, refs, now)?;
                        walk.report.imported += 1;
                    }
                }
                None => {
                    insert_note(connection, project, Some(number), &note, refs, now)?;
                    walk.report.created += 1;
                }
            }
            number
        }
        None => {
            let number = insert_note(connection, project, None, &note, refs, now)?;
            walk.seen.insert(number);
            walk.report.created += 1;
            number
        }
    };
    walk.locations.insert(number, file.to_path_buf());
    Ok(())
}

fn apply_edit(
    connection: &Connection,
    project: &str,
    number: i64,
    note: &ParsedNote,
    refs: Vec<MemoryRef>,
    now: &str,
) -> Result<()> {
    let mut memory = load(connection, project, number)?;
    if let Some(kind) = note.front.kind {
        memory.kind = kind;
    }
    memory.title = note.title.clone();
    memory.text = note.text.clone();
    memory.scope = scope_of(&note.front);
    merge_refs(&mut memory.refs, &refs);
    if let Some(confidence) = note.front.confidence {
        memory.confidence = confidence;
    }
    if let Some(status) = note.front.status {
        memory.status = status;
    }
    memory.supersedes = note.front.supersedes.clone();
    memory.updated_at = now.to_string();
    save(connection, &memory)?;
    if memory.status == MemoryStatus::Active {
        supersede_previous(connection, &memory, now)?;
    }
    Ok(())
}

/// Creates the memory of a note: at its own number when rebuilding, else at a new one.
fn insert_note(
    connection: &Connection,
    project: &str,
    wanted: Option<i64>,
    note: &ParsedNote,
    refs: Vec<MemoryRef>,
    now: &str,
) -> Result<i64> {
    if let Some(number) = wanted {
        let taken: i64 = connection.query_row(
            "SELECT COUNT(*) FROM memories WHERE num = ?1",
            params![number],
            |row| row.get(0),
        )?;
        if taken > 0 {
            return Err(MemoryError::Conflict(format!(
                "{} already belongs to another project",
                format_id(number)
            )));
        }
    }
    let kind = note.front.kind.unwrap_or(NOTE_DEFAULT_KIND);
    let status = note.front.status.unwrap_or(MemoryStatus::Active);
    let supersedes = note.front.supersedes.as_deref().map(parse_id).transpose()?;
    connection.execute(
        "INSERT INTO memories (num, project, kind, title, text, scope, refs, confidence, evidence, status, supersedes, created_at, updated_at, uses) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, 0)",
        params![
            wanted,
            project,
            kind.as_str(),
            note.title,
            note.text,
            serde_json::to_string(&scope_of(&note.front))?,
            serde_json::to_string(&refs)?,
            note.front.confidence.unwrap_or(DEFAULT_CONFIDENCE),
            note.front.evidence.unwrap_or(1),
            status.as_str(),
            supersedes,
            note.front.created.as_deref().unwrap_or(now),
            note.front.updated.as_deref().unwrap_or(now),
        ],
    )?;
    let number = connection.last_insert_rowid();
    save(connection, &load(connection, project, number)?)?;
    Ok(number)
}

fn scope_of(front: &MemoryNote) -> MemoryScope {
    MemoryScope {
        route: front.route,
        paths: front.paths.clone(),
        tags: front.tags.clone(),
    }
}

fn archive_deleted(
    connection: &Connection,
    project: &str,
    hashes: &HashMap<i64, Option<String>>,
    now: &str,
    walk: &mut Walk,
) -> Result<()> {
    for (number, hash) in hashes {
        if hash.is_none() || walk.seen.contains(number) {
            continue;
        }
        let mut memory = load(connection, project, *number)?;
        if memory.status != MemoryStatus::Archived {
            memory.status = MemoryStatus::Archived;
            memory.updated_at = now.to_string();
            save(connection, &memory)?;
            walk.report.archived += 1;
        }
    }
    Ok(())
}

fn export(connection: &Connection, project: &str, notes_dir: &Path, walk: &mut Walk) -> Result<()> {
    for memory in all_memories(connection, project)? {
        let number = parse_id(&memory.id)?;
        if walk.untouchable.contains(&number) {
            continue;
        }
        let content = render_note(&memory)?;
        let target = note_path(notes_dir, &memory);
        if let Some(previous) = walk.locations.get(&number).filter(|path| **path != target) {
            std::fs::remove_file(previous)?;
        }
        let current = std::fs::read_to_string(&target).ok();
        if current.as_deref() != Some(content.as_str()) {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&target, &content)?;
            walk.report.exported += 1;
        }
        connection.execute(
            "UPDATE memories SET note_hash = ?1 WHERE num = ?2",
            params![short_hash(&content), number],
        )?;
    }
    Ok(())
}

fn all_memories(connection: &Connection, project: &str) -> Result<Vec<Memory>> {
    let mut statement = connection.prepare(&format!(
        "SELECT {MEMORY_COLUMNS} FROM memories m WHERE m.project = ?1 ORDER BY m.num"
    ))?;
    let rows = statement.query_map(params![project], row_to_memory)?;
    Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
}

fn note_path(notes_dir: &Path, memory: &Memory) -> PathBuf {
    let folder = if matches!(
        memory.status,
        MemoryStatus::Archived | MemoryStatus::Superseded
    ) {
        notes_dir.join(ARCHIVE_DIR)
    } else {
        notes_dir
            .join(
                memory
                    .scope
                    .route
                    .map_or(NO_ROUTE_DIR, |route| route.as_str()),
            )
            .join(memory.kind.as_str())
    };
    folder.join(format!(
        "{}{SLUG_SEPARATOR}{}.{MARKDOWN_EXT}",
        memory.id,
        slug(&memory.title)
    ))
}

fn slug(title: &str) -> String {
    let separated: String = fold(title)
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                SLUG_SEPARATOR
            }
        })
        .collect();
    separated
        .split(SLUG_SEPARATOR)
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>()
        .join(&SLUG_SEPARATOR.to_string())
        .chars()
        .take(SLUG_MAX_CHARS)
        .collect::<String>()
        .trim_end_matches(SLUG_SEPARATOR)
        .to_string()
}

fn render_note(memory: &Memory) -> Result<String> {
    let front = MemoryNote {
        id: Some(memory.id.clone()),
        kind: Some(memory.kind),
        status: Some(memory.status),
        route: memory.scope.route,
        paths: memory.scope.paths.clone(),
        tags: memory.scope.tags.clone(),
        refs: memory
            .refs
            .iter()
            .map(|reference| format!("{}{NOTE_REF_SEPARATOR}{}", reference.ref_type, reference.id))
            .collect(),
        confidence: Some(memory.confidence),
        evidence: Some(memory.evidence),
        supersedes: memory.supersedes.clone(),
        created: Some(memory.created_at.clone()),
        updated: Some(memory.updated_at.clone()),
    };
    let yaml = serde_yaml::to_string(&front).map_err(invalid_note)?;
    Ok(format!(
        "{FRONTMATTER_FENCE}\n{yaml}{FRONTMATTER_FENCE}\n\n{TITLE_PREFIX}{}\n\n{}\n",
        memory.title, memory.text
    ))
}

fn parse_note(content: &str) -> Result<ParsedNote> {
    let normalized = content.replace("\r\n", "\n");
    let fence = format!("{FRONTMATTER_FENCE}\n");
    let rest = normalized
        .trim_start_matches('\u{feff}')
        .strip_prefix(&fence)
        .ok_or_else(|| MemoryError::Invalid("the note has no frontmatter".to_string()))?;
    let (yaml, body) = rest
        .split_once(&format!("\n{fence}"))
        .ok_or_else(|| MemoryError::Invalid("the frontmatter is not closed".to_string()))?;
    let front: MemoryNote = serde_yaml::from_str(yaml).map_err(invalid_note)?;
    let body = body.trim();
    let (heading, text) = body.split_once('\n').unwrap_or((body, ""));
    let title = heading
        .strip_prefix(TITLE_PREFIX)
        .ok_or_else(|| MemoryError::Invalid("the note has no `# title` line".to_string()))?;
    Ok(ParsedNote {
        front,
        title: title.trim().to_string(),
        text: text.trim().to_string(),
    })
}

fn parse_refs(raw: &[String]) -> Result<Vec<MemoryRef>> {
    raw.iter()
        .map(|entry| {
            let (kind, id) = entry.split_once(NOTE_REF_SEPARATOR).ok_or_else(|| {
                MemoryError::Invalid(format!("invalid reference `{entry}`; expected type:id"))
            })?;
            Ok(MemoryRef {
                ref_type: RefType::parse(kind)?,
                id: id.trim().to_string(),
            })
        })
        .collect()
}

fn invalid_note(error: serde_yaml::Error) -> MemoryError {
    MemoryError::Invalid(format!("invalid note frontmatter: {error}"))
}

#[cfg(test)]
#[path = "../tests/unit/vault_notes_tests.rs"]
mod tests;
