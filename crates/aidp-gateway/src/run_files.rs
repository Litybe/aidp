//! Read-only access to the files of a native run directory:
//! `GET /api/runs/:id/files` lists them, `GET /api/runs/:id/files/*path` serves one.
//! Every path is confined to the run directory; symbolic links are never followed.

use crate::models::constants::static_ui::{CACHE_REVALIDATE, MIME_DEFAULT, SEGMENT_SEPARATOR};
use crate::models::static_file::StaticFile;
use crate::native_runs::NativeRunManager;
use crate::static_ui::{decode_segments, mime_for};
use serde::Serialize;
use std::path::{Path, PathBuf};
use tiny_http::Method;

const API_PREFIX: &str = "/api";
const RUNS_SEGMENT: &str = "runs";
const FILES_SEGMENT: &str = "files";
/// Most entries one listing returns.
pub const MAX_ENTRIES: usize = 2000;
/// Largest file the read route serves, in bytes.
pub const MAX_FILE_BYTES: u64 = 20 * 1024 * 1024;
/// Deepest directory level the listing walks.
const MAX_DEPTH: usize = 32;
const STATUS_NOT_FOUND: u16 = 404;
const STATUS_TOO_LARGE: u16 = 413;
const NO_RUN: &str = "run not found";
const NO_FILE: &str = "file not found";
const TOO_LARGE: &str = "file too large";
const MIME_MARKDOWN: &str = "text/markdown; charset=utf-8";
const MIME_PLAIN: &str = "text/plain; charset=utf-8";

/// What the UI shows a file as.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum FileKind {
    Md,
    Json,
    Png,
    Gif,
    Other,
}

/// One row of the listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RunFile {
    /// Path relative to the run directory, `/` separated.
    pub path: String,
    pub size: u64,
    pub kind: FileKind,
    /// First directory of the path; empty for a file at the root of the run.
    pub stage: String,
}

/// The answer of a run files route.
#[derive(Debug)]
pub enum RunFilesReply {
    Json(u16, serde_json::Value),
    File(StaticFile),
    Error(u16, &'static str),
}

/// The kind of a file from its name.
pub fn kind_of(name: &str) -> FileKind {
    match extension(name).to_ascii_lowercase().as_str() {
        "md" => FileKind::Md,
        "json" => FileKind::Json,
        "png" => FileKind::Png,
        "gif" => FileKind::Gif,
        _ => FileKind::Other,
    }
}

fn extension(name: &str) -> &str {
    name.rsplit_once('.').map_or("", |(_, ext)| ext)
}

/// The content type of a file: markdown and YAML on top of the UI table.
pub fn content_type_of(name: &str) -> &'static str {
    let ext = extension(name);
    if ext.eq_ignore_ascii_case("md") {
        return MIME_MARKDOWN;
    }
    if ext.eq_ignore_ascii_case("yaml") || ext.eq_ignore_ascii_case("yml") {
        return MIME_PLAIN;
    }
    mime_for(ext).unwrap_or(MIME_DEFAULT)
}

/// True when `path` is a symbolic link or anything the OS reports as one.
fn is_link(path: &Path) -> bool {
    std::fs::symlink_metadata(path).map_or(true, |meta| meta.file_type().is_symlink())
}

fn walk(dir: &Path, relative: &str, depth: usize, out: &mut Vec<RunFile>) {
    if depth > MAX_DEPTH || out.len() >= MAX_ENTRIES {
        return;
    }
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<_> = read.filter_map(Result::ok).collect();
    entries.sort_by_key(std::fs::DirEntry::file_name);
    for entry in entries {
        if out.len() >= MAX_ENTRIES {
            return;
        }
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_symlink() {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let rel = if relative.is_empty() {
            name.clone()
        } else {
            format!("{relative}{SEGMENT_SEPARATOR}{name}")
        };
        if file_type.is_dir() {
            walk(&entry.path(), &rel, depth + 1, out);
        } else if file_type.is_file() {
            let size = entry.metadata().map_or(0, |meta| meta.len());
            let stage = rel
                .split_once(SEGMENT_SEPARATOR)
                .map_or_else(String::new, |(first, _)| first.to_string());
            out.push(RunFile {
                kind: kind_of(&name),
                path: rel,
                size,
                stage,
            });
        }
    }
}

/// Every regular file under `run_dir`, sorted by stage then path, at most [`MAX_ENTRIES`].
pub fn list_files(run_dir: &Path) -> Vec<RunFile> {
    let mut out = Vec::new();
    walk(run_dir, "", 0, &mut out);
    out.sort_by(|a, b| (&a.stage, &a.path).cmp(&(&b.stage, &b.path)));
    out
}

/// The canonical path of the regular file `segments` names under `run_dir`,
/// `None` when it is missing, escapes the directory or crosses a symbolic link.
fn resolve(run_dir: &Path, segments: &[String]) -> Option<PathBuf> {
    let root = run_dir.canonicalize().ok()?;
    let mut current = root.clone();
    for segment in segments {
        current.push(segment);
        if is_link(&current) {
            return None;
        }
    }
    let resolved = current.canonicalize().ok()?;
    (resolved.starts_with(&root) && resolved.is_file()).then_some(resolved)
}

/// Reads one file of the run directory; `path` is the raw (percent-encoded) relative path.
pub fn read_file(run_dir: &Path, path: &str) -> RunFilesReply {
    let Some(segments) = decode_segments(path).filter(|s| !s.is_empty()) else {
        return RunFilesReply::Error(STATUS_NOT_FOUND, NO_FILE);
    };
    let Some(resolved) = resolve(run_dir, &segments) else {
        return RunFilesReply::Error(STATUS_NOT_FOUND, NO_FILE);
    };
    let Ok(meta) = std::fs::metadata(&resolved) else {
        return RunFilesReply::Error(STATUS_NOT_FOUND, NO_FILE);
    };
    if meta.len() > MAX_FILE_BYTES {
        return RunFilesReply::Error(STATUS_TOO_LARGE, TOO_LARGE);
    }
    match std::fs::read(&resolved) {
        Ok(bytes) => RunFilesReply::File(StaticFile {
            bytes,
            content_type: content_type_of(segments.last().map_or("", String::as_str)),
            cache_control: CACHE_REVALIDATE,
            content_encoding: None,
        }),
        Err(_) => RunFilesReply::Error(STATUS_NOT_FOUND, NO_FILE),
    }
}

/// Answers `GET /api/runs/:id/files[/*path]`, or `None` for any other request.
pub fn handle(manager: &NativeRunManager, method: &Method, path: &str) -> Option<RunFilesReply> {
    if *method != Method::Get {
        return None;
    }
    let rest = path
        .strip_prefix(API_PREFIX)
        .unwrap_or(path)
        .trim_start_matches(SEGMENT_SEPARATOR);
    let mut parts = rest.splitn(4, SEGMENT_SEPARATOR);
    if parts.next()? != RUNS_SEGMENT {
        return None;
    }
    let run_id = crate::path_segments::decode(parts.next()?);
    if parts.next()? != FILES_SEGMENT {
        return None;
    }
    let file_path = parts.next().unwrap_or("");
    let Some(view) = manager.get(&run_id).filter(|view| view.run_dir.is_dir()) else {
        return Some(RunFilesReply::Error(STATUS_NOT_FOUND, NO_RUN));
    };
    if file_path.trim_matches(SEGMENT_SEPARATOR).is_empty() {
        let files = list_files(&view.run_dir);
        return Some(RunFilesReply::Json(
            200,
            serde_json::to_value(files).unwrap_or_default(),
        ));
    }
    Some(read_file(&view.run_dir, file_path))
}

#[cfg(test)]
#[path = "../tests/unit/run_files_tests.rs"]
mod tests;
