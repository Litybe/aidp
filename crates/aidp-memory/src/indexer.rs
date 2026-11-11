//! Incremental indexer: walks the sources, re-chunks changed files, drops vanished ones.

use crate::chunking::{csharp::chunk_csharp, markdown::chunk_markdown};
use crate::embedder::Embedder;
use crate::embedding_index::embed_pending;
use crate::error::{MemoryError, Result};
use crate::knowledge_note::parse_knowledge;
use crate::models::chunk_meta::ChunkMeta;
use crate::models::constants::limits::MAX_FILE_BYTES;
use crate::models::constants::paths::{
    BACKSLASH, BIBLE_DIR, CODE_DIR, CSHARP_EXT, DOC_DIR, EXCLUDED_DIRS, MARKDOWN_EXT, README_FILE,
    SEPARATOR, TEXT_EXTS,
};
use crate::models::constants::schema::GLOBAL_PROJECT;
use crate::models::enums::Source;
use crate::models::index_options::IndexOptions;
use crate::models::index_report::IndexReport;
use crate::models::index_root::IndexRoot;
use crate::models::knowledge_note::KnowledgeNote;
use crate::models::project_ref::ProjectRef;
use crate::models::raw_chunk::RawChunk;
use crate::store::MemoryStore;
use crate::text::{identifier_tokens, short_hash};
use rusqlite::{params, Transaction};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Instant, UNIX_EPOCH};
use walkdir::WalkDir;

const CHUNK_ID_SEPARATOR: &str = "#";

struct Target {
    dir: PathBuf,
    prefix: String,
    extensions: &'static [&'static str],
    only_file: Option<&'static str>,
}

struct StoredFile {
    hash: String,
    mtime: i64,
    size: i64,
}

/// Builds and refreshes the chunk index of a project.
pub struct Indexer<'a> {
    store: &'a MemoryStore,
    embedder: Option<&'a dyn Embedder>,
}

impl<'a> Indexer<'a> {
    pub fn new(store: &'a MemoryStore) -> Self {
        Self {
            store,
            embedder: None,
        }
    }

    /// Embeds new and changed chunks, and re-embeds those of another model, in the indexing transactions.
    pub fn with_embedder(mut self, embedder: &'a dyn Embedder) -> Self {
        self.embedder = Some(embedder);
        self
    }

    /// Refreshes the requested sources of `project` and reports what changed.
    pub fn index(&self, project: &ProjectRef, options: &IndexOptions) -> Result<IndexReport> {
        if !project.root.is_dir() {
            return Err(MemoryError::NotFound(format!(
                "project directory not found: {}",
                project.root.display()
            )));
        }
        let started = Instant::now();
        let sources: Vec<Source> = if options.sources.is_empty() {
            Source::FILE_SOURCES.to_vec()
        } else {
            options.sources.clone()
        };
        let mut report = IndexReport::default();
        for source in sources {
            if source == Source::Memory {
                continue;
            }
            let targets = targets_for(source, project, &options.extra_roots);
            if targets.is_empty() {
                report.skipped_sources.push(source.as_str().to_string());
                continue;
            }
            self.store.write(|transaction| {
                index_source(
                    transaction,
                    project,
                    source,
                    &targets,
                    options.full,
                    &mut report,
                )?;
                if let Some(embedder) = self.embedder {
                    report.embedded +=
                        embed_pending(transaction, embedder, &project.key, Some(source))?;
                }
                Ok(())
            })?;
        }
        self.index_global_knowledge(project, options, &mut report)?;
        if let Some(embedder) = self.embedder {
            report.embedded += self
                .store
                .write(|transaction| embed_pending(transaction, embedder, &project.key, None))?;
        }
        report.chunks = self.store.stats(&project.key)?.chunks;
        report.duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        Ok(report)
    }
}

impl Indexer<'_> {
    fn index_global_knowledge(
        &self,
        project: &ProjectRef,
        options: &IndexOptions,
        report: &mut IndexReport,
    ) -> Result<()> {
        let Some(root) = options.global_knowledge.as_ref() else {
            return Ok(());
        };
        let wanted = options.sources.is_empty() || options.sources.contains(&Source::Knowledge);
        if !wanted || project.key == GLOBAL_PROJECT || !root.dir.is_dir() {
            return Ok(());
        }
        let global = ProjectRef {
            key: GLOBAL_PROJECT.to_string(),
            root: project.root.clone(),
        };
        let targets = [Target {
            dir: root.dir.clone(),
            prefix: root.prefix.clone(),
            extensions: &[MARKDOWN_EXT],
            only_file: None,
        }];
        self.store.write(|transaction| {
            index_source(
                transaction,
                &global,
                Source::Knowledge,
                &targets,
                options.full,
                report,
            )?;
            if let Some(embedder) = self.embedder {
                report.embedded += embed_pending(
                    transaction,
                    embedder,
                    GLOBAL_PROJECT,
                    Some(Source::Knowledge),
                )?;
            }
            Ok(())
        })
    }
}

fn targets_for(source: Source, project: &ProjectRef, extra: &[IndexRoot]) -> Vec<Target> {
    let root = &project.root;
    match source {
        Source::Bible => vec![Target {
            dir: root.join(BIBLE_DIR),
            prefix: BIBLE_DIR.to_string(),
            extensions: &[MARKDOWN_EXT],
            only_file: None,
        }],
        Source::Doc => vec![
            Target {
                dir: root.join(DOC_DIR),
                prefix: DOC_DIR.to_string(),
                extensions: &[MARKDOWN_EXT],
                only_file: None,
            },
            Target {
                dir: root.clone(),
                prefix: String::new(),
                extensions: &[MARKDOWN_EXT],
                only_file: Some(README_FILE),
            },
        ],
        Source::Code => vec![Target {
            dir: root.join(CODE_DIR),
            prefix: CODE_DIR.to_string(),
            extensions: &[CSHARP_EXT],
            only_file: None,
        }],
        Source::Run | Source::Ticket | Source::Knowledge => extra
            .iter()
            .filter(|entry| entry.source == source)
            .map(|entry| Target {
                dir: entry.dir.clone(),
                prefix: entry.prefix.clone(),
                extensions: if source == Source::Run {
                    RUN_EXTENSIONS
                } else {
                    &[MARKDOWN_EXT]
                },
                only_file: None,
            })
            .collect(),
        Source::Memory => Vec::new(),
    }
}

const RUN_EXTENSIONS: &[&str] = &[MARKDOWN_EXT, TEXT_EXTS[0], TEXT_EXTS[1], TEXT_EXTS[2]];

fn index_source(
    transaction: &Transaction,
    project: &ProjectRef,
    source: Source,
    targets: &[Target],
    full: bool,
    report: &mut IndexReport,
) -> Result<()> {
    let mut stored = load_files(transaction, &project.key, source)?;
    let mut seen: HashSet<String> = HashSet::new();
    for target in targets {
        for (path, relative) in list_files(target) {
            let stored_path = join_prefix(&target.prefix, &relative);
            if !seen.insert(stored_path.clone()) {
                continue;
            }
            let metadata = match std::fs::metadata(&path) {
                Ok(metadata) if metadata.len() <= MAX_FILE_BYTES => metadata,
                _ => continue,
            };
            let mtime = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |elapsed| elapsed.as_secs() as i64);
            let size = metadata.len() as i64;
            let previous = stored.remove(&stored_path);
            if let Some(previous) = &previous {
                if !full && previous.mtime == mtime && previous.size == size {
                    report.unchanged += 1;
                    continue;
                }
            }
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            let content = String::from_utf8_lossy(&bytes);
            let hash = short_hash(&content);
            if let Some(previous) = &previous {
                if !full && previous.hash == hash {
                    transaction.execute(
                        "UPDATE files SET mtime = ?1, size = ?2 WHERE project = ?3 AND source = ?4 AND path = ?5",
                        params![mtime, size, project.key, source.as_str(), stored_path],
                    )?;
                    report.unchanged += 1;
                    continue;
                }
            }
            delete_chunks(transaction, &project.key, source, &stored_path)?;
            let (raw, note) = chunk_source_file(source, &path, &content);
            insert_chunks(
                transaction,
                project,
                source,
                &stored_path,
                mtime,
                &raw,
                note.as_ref(),
            )?;
            transaction.execute(
                "INSERT OR REPLACE INTO files (project, source, path, hash, mtime, size) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![project.key, source.as_str(), stored_path, hash, mtime, size],
            )?;
            if previous.is_some() {
                report.updated += 1;
            } else {
                report.indexed += 1;
            }
        }
    }
    for vanished in stored.keys() {
        delete_chunks(transaction, &project.key, source, vanished)?;
        transaction.execute(
            "DELETE FROM files WHERE project = ?1 AND source = ?2 AND path = ?3",
            params![project.key, source.as_str(), vanished],
        )?;
        report.removed += 1;
    }
    Ok(())
}

fn load_files(
    transaction: &Transaction,
    project: &str,
    source: Source,
) -> Result<HashMap<String, StoredFile>> {
    let mut statement = transaction
        .prepare("SELECT path, hash, mtime, size FROM files WHERE project = ?1 AND source = ?2")?;
    let rows = statement.query_map(params![project, source.as_str()], |row| {
        Ok((
            row.get::<_, String>(0)?,
            StoredFile {
                hash: row.get(1)?,
                mtime: row.get(2)?,
                size: row.get(3)?,
            },
        ))
    })?;
    let mut files = HashMap::new();
    for row in rows {
        let (path, file) = row?;
        files.insert(path, file);
    }
    Ok(files)
}

fn list_files(target: &Target) -> Vec<(PathBuf, String)> {
    let mut found = Vec::new();
    if !target.dir.is_dir() {
        return found;
    }
    let walker = WalkDir::new(&target.dir)
        .max_depth(if target.only_file.is_some() {
            1
        } else {
            usize::MAX
        })
        .follow_links(false)
        .into_iter()
        .filter_entry(|entry| {
            entry.depth() == 0
                || !entry.file_type().is_dir()
                || !EXCLUDED_DIRS.contains(&entry.file_name().to_string_lossy().as_ref())
        });
    for entry in walker.flatten() {
        if !entry.file_type().is_file() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        if let Some(only) = target.only_file {
            if name != only {
                continue;
            }
        }
        let extension = entry
            .path()
            .extension()
            .map(|ext| ext.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        if !target.extensions.contains(&extension.as_str()) {
            continue;
        }
        if let Ok(relative) = entry.path().strip_prefix(&target.dir) {
            found.push((entry.path().to_path_buf(), normalize(relative)));
        }
    }
    found.sort_by(|left, right| left.1.cmp(&right.1));
    found
}

fn normalize(path: &Path) -> String {
    path.to_string_lossy()
        .replace(BACKSLASH, &SEPARATOR.to_string())
}

fn join_prefix(prefix: &str, relative: &str) -> String {
    if prefix.is_empty() {
        relative.to_string()
    } else {
        format!("{prefix}{SEPARATOR}{relative}")
    }
}

fn chunk_file(path: &Path, content: &str) -> Vec<RawChunk> {
    let stem = path
        .file_stem()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();
    let is_code = path
        .extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case(CSHARP_EXT));
    if is_code {
        chunk_csharp(content, &stem)
    } else {
        chunk_markdown(content, &stem)
    }
}

fn chunk_source_file(
    source: Source,
    path: &Path,
    content: &str,
) -> (Vec<RawChunk>, Option<KnowledgeNote>) {
    let mut raw = chunk_file(path, content);
    if source != Source::Knowledge {
        return (raw, None);
    }
    let stem = path
        .file_stem()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();
    let note = parse_knowledge(content, &stem);
    if !note.is_indexable() {
        return (Vec::new(), None);
    }
    note.title_chunks(&mut raw, &stem);
    (raw, Some(note))
}

fn delete_chunks(
    transaction: &Transaction,
    project: &str,
    source: Source,
    path: &str,
) -> Result<()> {
    transaction.execute(
        "DELETE FROM chunks WHERE project = ?1 AND source = ?2 AND path = ?3",
        params![project, source.as_str(), path],
    )?;
    Ok(())
}

fn insert_chunks(
    transaction: &Transaction,
    project: &ProjectRef,
    source: Source,
    path: &str,
    mtime: i64,
    raw: &[RawChunk],
    note: Option<&KnowledgeNote>,
) -> Result<()> {
    let mut statement = transaction.prepare_cached(
        "INSERT OR REPLACE INTO chunks (id, project, source, path, line_start, line_end, title, text, idents, hash, mtime, meta) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
    )?;
    for (index, chunk) in raw.iter().enumerate() {
        let id = format!(
            "{}:{}:{path}{CHUNK_ID_SEPARATOR}{index}",
            project.key,
            source.as_str()
        );
        let idents = if source == Source::Code {
            identifier_tokens(&format!("{} {}", chunk.title, chunk.text))
        } else {
            String::new()
        };
        let meta = ChunkMeta {
            kind: chunk.kind.to_string(),
            symbol: chunk.symbol.clone(),
            domain: note.and_then(|note| note.front.domain.clone()),
            agents: note.map_or_else(Vec::new, |note| note.front.agents.clone()),
            status: note.and_then(|note| note.front.status.clone()),
            importance: note.and_then(|note| note.front.importance),
            tags: note.map_or_else(Vec::new, |note| note.front.tags.clone()),
        };
        statement.execute(params![
            id,
            project.key,
            source.as_str(),
            path,
            chunk.line_start as i64,
            chunk.line_end as i64,
            chunk.title,
            chunk.text,
            idents,
            short_hash(&chunk.text),
            mtime,
            serde_json::to_string(&meta)?,
        ])?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "../tests/unit/indexer_tests.rs"]
mod tests;
