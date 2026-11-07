//! Tiny file store: atomic JSON documents and append-only JSONL logs.

use serde::de::DeserializeOwned;
use serde::Serialize;
use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub fn ensure_dir(dir: &Path) {
    let _ = fs::create_dir_all(dir);
}

pub fn read_json<T: DeserializeOwned>(file: &Path, fallback: T) -> T {
    match fs::read_to_string(file) {
        Ok(txt) => serde_json::from_str(&txt).unwrap_or(fallback),
        Err(_) => fallback,
    }
}

/// Extension appended to every temporary document written by [`write_json`].
const TMP_EXTENSION: &str = "tmp";

/// Per-process sequence number making each [`write_json`] temp file unique.
static WRITE_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Numeric part of the current thread's id, without the `ThreadId(...)`
/// decoration, so it is safe inside a Windows file name.
fn current_thread_number() -> String {
    format!("{:?}", std::thread::current().id())
        .chars()
        .filter(char::is_ascii_digit)
        .collect()
}

/// Temp path next to `file`, unique per process, thread and write call.
fn unique_tmp_path(file: &Path) -> PathBuf {
    let sequence = WRITE_COUNTER.fetch_add(1, Ordering::Relaxed);
    let file_name = file.file_name().unwrap_or_default().to_string_lossy();
    let tmp_name = format!(
        "{file_name}.{}.{}.{sequence}.{TMP_EXTENSION}",
        std::process::id(),
        current_thread_number()
    );
    file.with_file_name(tmp_name)
}

/// tmp + rename so a crash never leaves a half-written document; the temp
/// name is unique per call, and a failed rename removes it. Windows rename
/// does not overwrite, so the target is removed first: the short window
/// where the file is absent is known and deliberate, since closing it
/// would need an unsafe platform call.
pub fn write_json<T: Serialize>(file: &Path, data: &T) -> anyhow::Result<()> {
    if let Some(dir) = file.parent() {
        ensure_dir(dir);
    }
    let tmp = unique_tmp_path(file);
    fs::write(&tmp, serde_json::to_string_pretty(data)?)?;
    if file.exists() {
        let _ = fs::remove_file(file);
    }
    if let Err(rename_error) = fs::rename(&tmp, file) {
        let _ = fs::remove_file(&tmp);
        return Err(rename_error.into());
    }
    Ok(())
}

/// Appends one record to a JSONL log; failures are logged, never returned.
pub fn append_jsonl<T: Serialize>(file: &Path, obj: &T) {
    if let Some(dir) = file.parent() {
        ensure_dir(dir);
    }
    let mut handle = match fs::OpenOptions::new().create(true).append(true).open(file) {
        Ok(handle) => handle,
        Err(error) => {
            warn_append_failure(file, &error);
            return;
        }
    };
    match serde_json::to_string(obj) {
        Ok(line) => {
            if let Err(error) = writeln!(handle, "{line}") {
                warn_append_failure(file, &error);
            }
        }
        Err(error) => warn_append_failure(file, &error),
    }
}

/// Message logged when a jsonl append fails.
pub(crate) const APPEND_FAILED_MESSAGE: &str = "append_jsonl failed";

fn warn_append_failure(file: &Path, error: &dyn std::fmt::Display) {
    tracing::warn!(
        path = %file.display(),
        error = %error,
        "{APPEND_FAILED_MESSAGE}"
    );
}

/// Most bytes [`read_jsonl_bounded`] scans back from the end of a log.
pub const MAX_SCAN_BYTES: u64 = 8 * 1024 * 1024;

/// Size of each backward read performed by [`read_jsonl_bounded`].
const CHUNK_SIZE: u64 = 64 * 1024;

const NEWLINE: u8 = b'\n';
const CARRIAGE_RETURN: &[u8] = b"\r";

/// Records read from a JSONL log, oldest first; `truncated` is true when
/// the scan stopped at [`MAX_SCAN_BYTES`] before the start of the file.
pub struct JsonlRead<T> {
    pub items: Vec<T>,
    pub truncated: bool,
}

/// Every record of a JSONL log, oldest first. Reads at most the last
/// [`MAX_SCAN_BYTES`] of the file; see [`read_jsonl_bounded`].
pub fn read_jsonl<T: DeserializeOwned>(file: &Path) -> Vec<T> {
    read_jsonl_bounded(file).items
}

/// Records of the last [`MAX_SCAN_BYTES`] of a JSONL log, oldest first,
/// scanned backwards in [`CHUNK_SIZE`] chunks. Blank and malformed lines
/// are skipped, a trailing `\r` is stripped, and when the window stops
/// before the start of the file its first (possibly partial) line is
/// dropped. A missing or unreadable file yields no items.
pub fn read_jsonl_bounded<T: DeserializeOwned>(file: &Path) -> JsonlRead<T> {
    let empty = || JsonlRead {
        items: Vec::new(),
        truncated: false,
    };
    let Ok(mut handle) = fs::File::open(file) else {
        return empty();
    };
    let Ok(metadata) = handle.metadata() else {
        return empty();
    };
    let mut newest_first: Vec<T> = Vec::new();
    let mut carry: Vec<u8> = Vec::new();
    let mut position = metadata.len();
    let mut scanned: u64 = 0;
    while position > 0 && scanned < MAX_SCAN_BYTES {
        let read_len = CHUNK_SIZE.min(position);
        let start = position - read_len;
        let Some(mut chunk) = read_chunk(&mut handle, start, read_len) else {
            break;
        };
        scanned += read_len;
        position = start;
        chunk.extend_from_slice(&carry);
        let complete_region = if start > 0 {
            let Some(first_newline) = chunk.iter().position(|&byte| byte == NEWLINE) else {
                carry = chunk;
                continue;
            };
            carry = chunk[..first_newline].to_vec();
            chunk.split_off(first_newline + 1)
        } else {
            carry.clear();
            chunk
        };
        collect_lines_newest_first(&complete_region, &mut newest_first);
    }
    newest_first.reverse();
    JsonlRead {
        items: newest_first,
        truncated: position > 0,
    }
}

/// Reads `len` bytes of `handle` starting at `start`, or `None` on any I/O error.
fn read_chunk(handle: &mut fs::File, start: u64, len: u64) -> Option<Vec<u8>> {
    handle.seek(SeekFrom::Start(start)).ok()?;
    let mut buffer = vec![0u8; usize::try_from(len).ok()?];
    handle.read_exact(&mut buffer).ok()?;
    Some(buffer)
}

/// Parses the complete lines of `region` from last to first into `sink`.
fn collect_lines_newest_first<T: DeserializeOwned>(region: &[u8], sink: &mut Vec<T>) {
    for line in region.split(|&byte| byte == NEWLINE).rev() {
        let line = line.strip_suffix(CARRIAGE_RETURN).unwrap_or(line);
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        if let Ok(item) = serde_json::from_slice::<T>(line) {
            sink.push(item);
        }
    }
}

/// Recursive size of `dir`: `(total bytes, file count)`. Never panics —
/// an unreadable directory or entry (permissions, a broken symlink, a
/// concurrent delete) is simply skipped rather than propagated, since this
/// exists only to report an approximate state-dir footprint for
/// `GET /api/health`.
pub fn dir_size(dir: &Path) -> (u64, usize) {
    let mut bytes: u64 = 0;
    let mut files: usize = 0;
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return (0, 0),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.is_dir() {
            let (b, f) = dir_size(&path);
            bytes += b;
            files += f;
        } else if meta.is_file() {
            bytes += meta.len();
            files += 1;
        }
        // Anything else (symlink to nowhere, special file) is skipped.
    }
    (bytes, files)
}

#[cfg(test)]
#[path = "../tests/unit/store_tests.rs"]
mod store_tests;
