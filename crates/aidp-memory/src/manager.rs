//! The memory manager: CRUD, life cycle, de-duplication, garbage collection.

use crate::error::{MemoryError, Result};
use crate::models::add_outcome::AddOutcome;
use crate::models::constants::limits::{
    AUTO_ACTIVATE_CONFIDENCE, AUTO_ACTIVATE_EVIDENCE, AUTO_ACTIVATE_ISSUES, DEDUP_CANDIDATES,
    DEDUP_SIMILARITY, STALE_AFTER_DAYS, TEXT_MAX_CHARS, TITLE_MAX_CHARS,
};
use crate::models::constants::schema::{
    MEMORY_CHUNK_PREFIX, MEMORY_ID_PREFIX, MEMORY_PATH_PREFIX, MEMORY_WEIGHTS,
};
use crate::models::enums::{MemoryKind, MemoryStatus, RefType, Source};
use crate::models::memory::Memory;
use crate::models::memory_filter::MemoryFilter;
use crate::models::memory_patch::MemoryPatch;
use crate::models::memory_ref::MemoryRef;
use crate::models::new_memory::NewMemory;
use crate::store::MemoryStore;
use crate::text::{jaccard, match_expression, short_hash, word_set};
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use rusqlite::{params, Connection};

pub(crate) const MEMORY_COLUMNS: &str = "m.num, m.project, m.kind, m.title, m.text, m.scope, m.refs, \
     m.confidence, m.evidence, m.status, m.supersedes, m.created_at, m.updated_at, m.last_used_at, m.uses";

type Clock = Box<dyn Fn() -> DateTime<Utc> + Send + Sync>;

/// Creates, reviews and retires the memories of projects.
pub struct MemoryManager<'a> {
    pub(crate) store: &'a MemoryStore,
    clock: Clock,
}

impl<'a> MemoryManager<'a> {
    /// A manager reading the system clock.
    pub fn new(store: &'a MemoryStore) -> Self {
        Self::with_clock(store, Box::new(Utc::now))
    }

    /// A manager with a custom clock, for tests.
    pub fn with_clock(store: &'a MemoryStore, clock: Clock) -> Self {
        Self { store, clock }
    }

    pub(crate) fn now(&self) -> String {
        format_time((self.clock)())
    }

    /// Adds a memory, or grows the evidence of the one a proposal repeats.
    pub fn add(&self, new: NewMemory) -> Result<AddOutcome> {
        validate_text(&new.title, &new.text, new.confidence)?;
        if !matches!(new.status, MemoryStatus::Proposed | MemoryStatus::Active) {
            return Err(MemoryError::Invalid(
                "a new memory is either proposed or active".to_string(),
            ));
        }
        let now = self.now();
        let outcome = self.store.write(|connection| {
            let supersedes = match &new.supersedes {
                Some(raw) => {
                    let number = parse_id(raw)?;
                    load(connection, &new.project, number)?;
                    Some(number)
                }
                None => None,
            };
            if new.status == MemoryStatus::Proposed {
                if let Some(mut existing) = find_duplicate(connection, &new)? {
                    existing.evidence += 1;
                    existing.confidence = existing.confidence.max(new.confidence);
                    merge_refs(&mut existing.refs, &new.refs);
                    existing.updated_at = now.clone();
                    let activated = maybe_activate(connection, &mut existing)?;
                    save(connection, &existing)?;
                    sync_chunks(connection, &existing.project)?;
                    return Ok(AddOutcome {
                        memory: existing,
                        deduplicated: true,
                        activated,
                    });
                }
            }
            let scope = serde_json::to_string(&new.scope)?;
            let refs = serde_json::to_string(&new.refs)?;
            connection.execute(
                "INSERT INTO memories (project, kind, title, text, scope, refs, confidence, evidence, status, supersedes, created_at, updated_at, uses) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8, ?9, ?10, ?10, 0)",
                params![
                    new.project,
                    new.kind.as_str(),
                    new.title,
                    new.text,
                    scope,
                    refs,
                    new.confidence,
                    new.status.as_str(),
                    supersedes,
                    now
                ],
            )?;
            let number = connection.last_insert_rowid();
            let memory = load(connection, &new.project, number)?;
            index_fts(connection, &memory)?;
            if memory.status == MemoryStatus::Active {
                supersede_previous(connection, &memory, &now)?;
            }
            sync_chunks(connection, &memory.project)?;
            Ok(AddOutcome {
                activated: memory.status == MemoryStatus::Active,
                memory,
                deduplicated: false,
            })
        })?;
        Ok(outcome)
    }

    /// One memory by id such as `M-12`.
    pub fn get(&self, project: &str, id: &str) -> Result<Memory> {
        let number = parse_id(id)?;
        self.store
            .read(|connection| load(connection, project, number))
    }

    /// Memories of a project in id order, filtered.
    pub fn list(&self, project: &str, filter: &MemoryFilter) -> Result<Vec<Memory>> {
        self.store.read(|connection| {
            let mut sql = format!("SELECT {MEMORY_COLUMNS} FROM memories m WHERE m.project = ?1");
            if let Some(status) = filter.status {
                sql.push_str(&format!(" AND m.status = '{}'", status.as_str()));
            }
            if let Some(kind) = filter.kind {
                sql.push_str(&format!(" AND m.kind = '{}'", kind.as_str()));
            }
            sql.push_str(" ORDER BY m.num");
            let mut statement = connection.prepare(&sql)?;
            let rows = statement.query_map(params![project], row_to_memory)?;
            let mut memories = rows.collect::<std::result::Result<Vec<_>, _>>()?;
            if let Some(query) = filter.query.as_deref().filter(|q| !q.trim().is_empty()) {
                let words = word_set(query);
                memories.retain(|memory| {
                    let haystack = word_set(&format!("{} {}", memory.title, memory.text));
                    words.iter().all(|word| {
                        haystack
                            .iter()
                            .any(|candidate| candidate.starts_with(word.as_str()))
                    })
                });
            }
            Ok(memories)
        })
    }

    /// Changes the editable fields of a memory.
    pub fn edit(&self, project: &str, id: &str, patch: &MemoryPatch) -> Result<Memory> {
        let number = parse_id(id)?;
        let now = self.now();
        self.store.write(|connection| {
            let mut memory = load(connection, project, number)?;
            if let Some(kind) = patch.kind {
                memory.kind = kind;
            }
            if let Some(title) = &patch.title {
                memory.title = title.clone();
            }
            if let Some(text) = &patch.text {
                memory.text = text.clone();
            }
            if let Some(route) = patch.route {
                memory.scope.route = Some(route);
            }
            if let Some(paths) = &patch.paths {
                memory.scope.paths = paths.clone();
            }
            if let Some(tags) = &patch.tags {
                memory.scope.tags = tags.clone();
            }
            if let Some(confidence) = patch.confidence {
                memory.confidence = confidence;
            }
            validate_text(&memory.title, &memory.text, memory.confidence)?;
            memory.updated_at = now.clone();
            save(connection, &memory)?;
            sync_chunks(connection, project)?;
            Ok(memory)
        })
    }

    /// Activates a proposed or stale memory; the one it supersedes is retired.
    pub fn approve(&self, project: &str, id: &str) -> Result<Memory> {
        let number = parse_id(id)?;
        let now = self.now();
        self.store.write(|connection| {
            let mut memory = load(connection, project, number)?;
            match memory.status {
                MemoryStatus::Active => return Ok(memory),
                MemoryStatus::Proposed | MemoryStatus::Stale => {}
                other => {
                    return Err(MemoryError::Conflict(format!(
                        "{} is {other}; only a proposed or stale memory can be approved",
                        memory.id
                    )))
                }
            }
            memory.status = MemoryStatus::Active;
            memory.updated_at = now.clone();
            save(connection, &memory)?;
            supersede_previous(connection, &memory, &now)?;
            sync_chunks(connection, project)?;
            Ok(memory)
        })
    }

    /// Archives a memory, whatever its state.
    pub fn archive(&self, project: &str, id: &str) -> Result<Memory> {
        let number = parse_id(id)?;
        let now = self.now();
        self.store.write(|connection| {
            let mut memory = load(connection, project, number)?;
            if memory.status != MemoryStatus::Archived {
                memory.status = MemoryStatus::Archived;
                memory.updated_at = now.clone();
                save(connection, &memory)?;
                sync_chunks(connection, project)?;
            }
            Ok(memory)
        })
    }

    /// Folds `id` into `into`: evidence, references and scope add up, `id` is superseded.
    pub fn merge(&self, project: &str, id: &str, into: &str) -> Result<Memory> {
        let source_number = parse_id(id)?;
        let target_number = parse_id(into)?;
        if source_number == target_number {
            return Err(MemoryError::Invalid(
                "a memory cannot be merged into itself".to_string(),
            ));
        }
        let now = self.now();
        self.store.write(|connection| {
            let mut source = load(connection, project, source_number)?;
            let mut target = load(connection, project, target_number)?;
            if matches!(
                target.status,
                MemoryStatus::Archived | MemoryStatus::Superseded
            ) {
                return Err(MemoryError::Conflict(format!(
                    "{} is {}; merge into a live memory",
                    target.id, target.status
                )));
            }
            target.evidence += source.evidence;
            target.confidence = target.confidence.max(source.confidence);
            merge_refs(&mut target.refs, &source.refs);
            union(&mut target.scope.paths, &source.scope.paths);
            union(&mut target.scope.tags, &source.scope.tags);
            if target.supersedes.is_none() {
                target.supersedes = Some(source.id.clone());
            }
            target.updated_at = now.clone();
            source.status = MemoryStatus::Superseded;
            source.updated_at = now.clone();
            maybe_activate(connection, &mut target)?;
            save(connection, &source)?;
            save(connection, &target)?;
            sync_chunks(connection, project)?;
            Ok(target)
        })
    }

    /// Marks stale the active memories unused for the default 60 days.
    pub fn gc(&self, project: &str) -> Result<Vec<Memory>> {
        self.gc_after(project, STALE_AFTER_DAYS)
    }

    /// Marks stale the active memories unused for `days` days; returns them.
    pub fn gc_after(&self, project: &str, days: i64) -> Result<Vec<Memory>> {
        let cutoff = format_time((self.clock)() - Duration::days(days));
        let now = self.now();
        self.store.write(|connection| {
            let mut statement = connection.prepare(&format!(
                "SELECT {MEMORY_COLUMNS} FROM memories m WHERE m.project = ?1 AND m.status = 'active' \
                 AND MAX(COALESCE(m.last_used_at, ''), m.updated_at) < ?2 ORDER BY m.num"
            ))?;
            let rows = statement.query_map(params![project, cutoff], row_to_memory)?;
            let mut stale = rows.collect::<std::result::Result<Vec<_>, _>>()?;
            drop(statement);
            for memory in &mut stale {
                memory.status = MemoryStatus::Stale;
                memory.updated_at = now.clone();
                save(connection, memory)?;
            }
            if !stale.is_empty() {
                sync_chunks(connection, project)?;
            }
            Ok(stale)
        })
    }
}

pub(crate) fn format_time(time: DateTime<Utc>) -> String {
    time.to_rfc3339_opts(SecondsFormat::Secs, true)
}

pub(crate) fn format_id(number: i64) -> String {
    format!("{MEMORY_ID_PREFIX}{number}")
}

pub(crate) fn parse_id(raw: &str) -> Result<i64> {
    let trimmed = raw.trim();
    let digits = if trimmed.len() >= MEMORY_ID_PREFIX.len()
        && trimmed[..MEMORY_ID_PREFIX.len()].eq_ignore_ascii_case(MEMORY_ID_PREFIX)
    {
        &trimmed[MEMORY_ID_PREFIX.len()..]
    } else {
        trimmed
    };
    digits.parse::<i64>().map_err(|_| {
        MemoryError::Invalid(format!("invalid memory id `{raw}`; expected M-<number>"))
    })
}

fn validate_text(title: &str, text: &str, confidence: f64) -> Result<()> {
    if title.trim().is_empty() || text.trim().is_empty() {
        return Err(MemoryError::Invalid(
            "title and text are required".to_string(),
        ));
    }
    if title.chars().count() > TITLE_MAX_CHARS {
        return Err(MemoryError::Invalid(format!(
            "title is longer than {TITLE_MAX_CHARS} characters"
        )));
    }
    if text.chars().count() > TEXT_MAX_CHARS {
        return Err(MemoryError::Invalid(format!(
            "text is longer than {TEXT_MAX_CHARS} characters"
        )));
    }
    if !(0.0..=1.0).contains(&confidence) {
        return Err(MemoryError::Invalid(
            "confidence must be between 0 and 1".to_string(),
        ));
    }
    Ok(())
}

pub(crate) fn row_to_memory(row: &rusqlite::Row) -> rusqlite::Result<Memory> {
    let convert = |index: usize, error: MemoryError| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    };
    let kind: String = row.get(2)?;
    let status: String = row.get(9)?;
    let scope: String = row.get(5)?;
    let refs: String = row.get(6)?;
    let supersedes: Option<i64> = row.get(10)?;
    Ok(Memory {
        id: format_id(row.get(0)?),
        project: row.get(1)?,
        kind: MemoryKind::parse(&kind).map_err(|error| convert(2, error))?,
        title: row.get(3)?,
        text: row.get(4)?,
        scope: serde_json::from_str(&scope).map_err(|error| convert(5, error.into()))?,
        refs: serde_json::from_str(&refs).map_err(|error| convert(6, error.into()))?,
        confidence: row.get(7)?,
        evidence: row.get(8)?,
        status: MemoryStatus::parse(&status).map_err(|error| convert(9, error))?,
        supersedes: supersedes.map(format_id),
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
        last_used_at: row.get(13)?,
        uses: row.get(14)?,
    })
}

pub(crate) fn load(connection: &Connection, project: &str, number: i64) -> Result<Memory> {
    connection
        .query_row(
            &format!("SELECT {MEMORY_COLUMNS} FROM memories m WHERE m.project = ?1 AND m.num = ?2"),
            params![project, number],
            row_to_memory,
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => MemoryError::NotFound(format!(
                "memory {} not found in project {project}",
                format_id(number)
            )),
            other => other.into(),
        })
}

pub(crate) fn save(connection: &Connection, memory: &Memory) -> Result<()> {
    let number = parse_id(&memory.id)?;
    let supersedes = memory.supersedes.as_deref().map(parse_id).transpose()?;
    connection.execute(
        "UPDATE memories SET kind = ?1, title = ?2, text = ?3, scope = ?4, refs = ?5, confidence = ?6, \
         evidence = ?7, status = ?8, supersedes = ?9, updated_at = ?10, last_used_at = ?11, uses = ?12 \
         WHERE num = ?13",
        params![
            memory.kind.as_str(),
            memory.title,
            memory.text,
            serde_json::to_string(&memory.scope)?,
            serde_json::to_string(&memory.refs)?,
            memory.confidence,
            memory.evidence,
            memory.status.as_str(),
            supersedes,
            memory.updated_at,
            memory.last_used_at,
            memory.uses,
            number
        ],
    )?;
    index_fts(connection, memory)
}

fn index_fts(connection: &Connection, memory: &Memory) -> Result<()> {
    let number = parse_id(&memory.id)?;
    connection.execute("DELETE FROM memories_fts WHERE rowid = ?1", params![number])?;
    connection.execute(
        "INSERT INTO memories_fts (rowid, title, text, tags) VALUES (?1, ?2, ?3, ?4)",
        params![
            number,
            memory.title,
            memory.text,
            memory.scope.tags.join(" ")
        ],
    )?;
    Ok(())
}

fn find_duplicate(connection: &Connection, new: &NewMemory) -> Result<Option<Memory>> {
    let combined = format!("{} {}", new.title, new.text);
    let Some(expression) = match_expression(&combined) else {
        return Ok(None);
    };
    let sql = format!(
        "SELECT {MEMORY_COLUMNS} FROM memories_fts JOIN memories m ON m.num = memories_fts.rowid \
         WHERE memories_fts MATCH ?1 AND m.project = ?2 AND m.status IN ('proposed', 'active', 'stale') \
         ORDER BY bm25(memories_fts, {MEMORY_WEIGHTS}) LIMIT ?3"
    );
    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map(
        params![expression, new.project, DEDUP_CANDIDATES as i64],
        row_to_memory,
    )?;
    let wanted = word_set(&combined);
    let mut best: Option<(f64, Memory)> = None;
    for row in rows {
        let memory = row?;
        let similarity = jaccard(
            &wanted,
            &word_set(&format!("{} {}", memory.title, memory.text)),
        );
        if similarity >= DEDUP_SIMILARITY && best.as_ref().is_none_or(|(top, _)| similarity > *top)
        {
            best = Some((similarity, memory));
        }
    }
    Ok(best.map(|(_, memory)| memory))
}

fn maybe_activate(connection: &Connection, memory: &mut Memory) -> Result<bool> {
    if memory.status != MemoryStatus::Proposed {
        return Ok(false);
    }
    let mut issues: Vec<&str> = memory
        .refs
        .iter()
        .filter(|reference| reference.ref_type == RefType::Issue)
        .map(|reference| reference.id.as_str())
        .collect();
    issues.sort_unstable();
    issues.dedup();
    if memory.evidence >= AUTO_ACTIVATE_EVIDENCE
        && issues.len() >= AUTO_ACTIVATE_ISSUES
        && memory.confidence >= AUTO_ACTIVATE_CONFIDENCE
    {
        memory.status = MemoryStatus::Active;
        let now = memory.updated_at.clone();
        supersede_previous(connection, memory, &now)?;
        return Ok(true);
    }
    Ok(false)
}

fn supersede_previous(connection: &Connection, memory: &Memory, now: &str) -> Result<()> {
    let Some(previous) = &memory.supersedes else {
        return Ok(());
    };
    let number = parse_id(previous)?;
    let Ok(mut old) = load(connection, &memory.project, number) else {
        return Ok(());
    };
    if old.id != memory.id && old.status != MemoryStatus::Superseded {
        old.status = MemoryStatus::Superseded;
        old.updated_at = now.to_string();
        save(connection, &old)?;
    }
    Ok(())
}

fn merge_refs(target: &mut Vec<MemoryRef>, extra: &[MemoryRef]) {
    for reference in extra {
        if !target.contains(reference) {
            target.push(reference.clone());
        }
    }
}

fn union(target: &mut Vec<String>, extra: &[String]) {
    for item in extra {
        if !target.contains(item) {
            target.push(item.clone());
        }
    }
}

/// Rewrites the `memory` chunks of a project from its active memories.
pub(crate) fn sync_chunks(connection: &Connection, project: &str) -> Result<()> {
    connection.execute(
        "DELETE FROM chunks WHERE project = ?1 AND source = ?2",
        params![project, Source::Memory.as_str()],
    )?;
    let mut statement = connection.prepare(&format!(
        "SELECT {MEMORY_COLUMNS} FROM memories m WHERE m.project = ?1 AND m.status = 'active' ORDER BY m.num"
    ))?;
    let rows = statement.query_map(params![project], row_to_memory)?;
    let memories = rows.collect::<std::result::Result<Vec<_>, _>>()?;
    drop(statement);
    for memory in memories {
        let text = format!("{}\n{}", memory.title, memory.text);
        connection.execute(
            "INSERT OR REPLACE INTO chunks (id, project, source, path, line_start, line_end, title, text, idents, hash, mtime, meta) \
             VALUES (?1, ?2, ?3, ?4, 1, 1, ?5, ?6, ?7, ?8, 0, '{}')",
            params![
                format!("{MEMORY_CHUNK_PREFIX}{}", memory.id),
                project,
                Source::Memory.as_str(),
                format!("{MEMORY_PATH_PREFIX}{}", memory.id),
                memory.title,
                text,
                memory.scope.tags.join(" "),
                short_hash(&text),
            ],
        )?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "../tests/unit/manager_tests.rs"]
mod tests;
