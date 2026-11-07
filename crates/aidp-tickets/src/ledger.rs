//! Append-only transition journal: `<vault>/TODO/.state/ticket-events.jsonl`
//! (vision §6: "every transition is journalled").
//!
//! **The actor rule (fixed once, here, instead of at every call site):** in
//! this lot only two of the four [`crate::model::Actor`] variants are ever
//! produced. `Actor::Human` is used for exactly the two verbs a person
//! invokes to force a transition — `aidp ticket status` and
//! `aidp ticket cancel` — because those are the only verbs where a human is
//! the one asking for the transition. Every other verb that moves a ticket
//! (`new`, `route`, `edit`, `reindex`'s archive sweep, …) journals
//! `Actor::Cli`. `Actor::Dispatcher` and `Actor::Sync` are reserved for the
//! dispatcher (lot 9a) and the Jira sync (lot 8a); nothing in this lot ever
//! constructs them.

use crate::model::{Actor, Status};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{Read as _, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// One journalled transition. `from` is `None` for a ticket's very first
/// event (creation has no prior status); every other field is required.
/// `reason` and `session_id` use `skip_serializing_if` (alongside `from`) so
/// the common case — a `Cli`-actor transition with no reason and no session —
/// stays a short line.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Event {
    pub ts: DateTime<Utc>,
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<Status>,
    pub to: Status,
    pub actor: Actor,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

fn state_dir(vault_root: &Path) -> PathBuf {
    vault_root.join("TODO").join(".state")
}

fn events_path(vault_root: &Path) -> PathBuf {
    state_dir(vault_root).join("ticket-events.jsonl")
}

/// Append `event` as one JSON line. Creates `TODO/.state/` if needed. Best
/// effort, mirroring `append_jsonl` in `aidp-gateway/src/store.rs`: any
/// failure (can't create the directory, can't open the file, can't
/// serialise) is reported on stderr and swallowed — a journalling problem
/// must never fail the caller's own write.
pub fn append(vault_root: &Path, event: &Event) {
    let dir = state_dir(vault_root);
    if let Err(e) = fs::create_dir_all(&dir) {
        tracing::warn!("failed to create {}: {e}", dir.display());
        return;
    }
    let file = events_path(vault_root);
    let mut f = match fs::OpenOptions::new().create(true).append(true).open(&file) {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!("failed to open {}: {e}", file.display());
            return;
        }
    };
    let line = match serde_json::to_string(event) {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!("failed to serialise event: {e}");
            return;
        }
    };
    if let Err(e) = writeln!(f, "{line}") {
        tracing::warn!("failed to append to {}: {e}", file.display());
    }
}

/// Hard ceiling on bytes scanned by [`read`] as it walks the journal
/// backwards from its end. The ceiling bounds bytes **scanned**, not events
/// **returned**, and that cuts two different ways:
///
/// - The result is exact as long as the requested `limit` events are
///   reachable within the last `MAX_SCAN_BYTES` of the journal. That holds
///   for any realistic `--limit` (the CLI's default is 50), but not for an
///   arbitrarily large one run against a journal that has already grown
///   past the ceiling: the scan still stops at `MAX_SCAN_BYTES`, and the
///   short result it returns then looks no different from "the journal
///   only holds this many events".
/// - With a `--key` filter over a journal bigger than `MAX_SCAN_BYTES`,
///   `read` reports only the matching events that lie within this trailing
///   window — a match further back in the file, beyond the window, is not
///   seen and is silently absent from the result.
///
/// Either way, the journal file itself is never truncated, rotated, or
/// rewritten by this function: `ticket-events.jsonl` stays the complete,
/// append-only history described in `docs/tickets.md`.
const MAX_SCAN_BYTES: u64 = 8 * 1024 * 1024;

/// Read back the journal, most recent last, optionally filtered to one
/// `key`, capped to the last `limit` matching events.
///
/// Cost is bounded rather than proportional to the journal's size: the file
/// is scanned **backwards** from its end in fixed-size chunks, carrying the
/// partial line at the front of each chunk over into the next one so a
/// record straddling a chunk boundary is never split. The scan stops as
/// soon as `limit` matching events have been collected, the start of the
/// file is reached, or [`MAX_SCAN_BYTES`] have been scanned — whichever
/// comes first; see that constant for exactly what the ceiling does and
/// does not guarantee.
///
/// A missing or unreadable file yields an empty vec, never an error;
/// `limit == 0` also yields an empty vec without touching the file. A
/// malformed line (partial write, hand-edit typo) is skipped, never fatal —
/// the file is append-only and chronological, so skipping a bad line does
/// not disturb the order of the rest. A trailing `\r` before a line's `\n`
/// (a CRLF-written or hand-edited line) is stripped rather than left to
/// break parsing.
pub fn read(vault_root: &Path, key: Option<&str>, limit: usize) -> Vec<Event> {
    if limit == 0 {
        return Vec::new();
    }

    let file_path = events_path(vault_root);
    let mut file = match fs::File::open(&file_path) {
        Ok(f) => f,
        Err(_) => return Vec::new(),
    };
    let len = match file.metadata() {
        Ok(m) => m.len(),
        Err(_) => return Vec::new(),
    };

    const CHUNK_SIZE: u64 = 64 * 1024;

    // Collected newest-first while the scan walks backward; reversed once
    // at the end to the chronological, oldest-first order callers expect.
    let mut matched: Vec<Event> = Vec::new();
    // The leading fragment of a chunk, up to (not including) its first
    // `\n`: the start of a line whose earlier bytes have not been read yet.
    // Carried into the next (earlier) chunk and prepended there — literally
    // spliced onto the front of the next raw read — so the line is
    // reconstructed whole instead of being split at the chunk boundary.
    let mut carry: Vec<u8> = Vec::new();
    let mut pos = len;
    let mut scanned: u64 = 0;

    'scan: while pos > 0 && matched.len() < limit && scanned < MAX_SCAN_BYTES {
        let read_len = CHUNK_SIZE.min(pos);
        let start = pos - read_len;
        scanned += read_len;

        if file.seek(SeekFrom::Start(start)).is_err() {
            break;
        }
        let mut buf = vec![0u8; read_len as usize];
        if file.read_exact(&mut buf).is_err() {
            break;
        }
        buf.extend_from_slice(&carry);
        pos = start;

        // Everything up to the first `\n` in `buf` is the start of a line
        // whose earlier bytes live before `start` — unless `start` is 0, in
        // which case there is nothing earlier and it is a complete first
        // line instead.
        let complete_region: &[u8] = if start > 0 {
            match buf.iter().position(|&b| b == b'\n') {
                Some(idx) => {
                    carry = buf[..idx].to_vec();
                    &buf[idx + 1..]
                }
                None => {
                    // No newline anywhere in this chunk: it is entirely the
                    // tail of one line that straddles more than one chunk.
                    // Carry it all forward and keep reading further back.
                    carry = buf;
                    continue 'scan;
                }
            }
        } else {
            carry = Vec::new();
            &buf[..]
        };

        for line in complete_region.split(|&b| b == b'\n').rev() {
            if matched.len() >= limit {
                break;
            }
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            if line.iter().all(u8::is_ascii_whitespace) {
                continue;
            }
            let Ok(event) = serde_json::from_slice::<Event>(line) else {
                continue;
            };
            if let Some(k) = key {
                if event.key != k {
                    continue;
                }
            }
            matched.push(event);
        }
    }

    matched.reverse();
    matched
}

#[cfg(test)]
mod tests {
    use super::*;
    use aidp_testkit::TempWorkspace;

    fn event(key: &str, to: Status) -> Event {
        Event {
            ts: Utc::now(),
            key: key.to_string(),
            from: Some(Status::Todo),
            to,
            actor: Actor::Cli,
            reason: None,
            session_id: None,
        }
    }

    #[test]
    fn append_then_read_round_trip() {
        let _workspace = TempWorkspace::new("roundtrip");
        let vault = _workspace.path();
        append(vault, &event("LOC-1", Status::Queued));

        let events = read(vault, None, 10);

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].key, "LOC-1");
        assert_eq!(events[0].from, Some(Status::Todo));
        assert_eq!(events[0].to, Status::Queued);
        assert_eq!(events[0].actor, Actor::Cli);
    }

    #[test]
    fn read_filters_by_key() {
        let _workspace = TempWorkspace::new("filter");
        let vault = _workspace.path();
        append(vault, &event("LOC-1", Status::Queued));
        append(vault, &event("LOC-2", Status::Queued));
        append(vault, &event("LOC-1", Status::InProgress));

        let events = read(vault, Some("LOC-1"), 10);

        assert_eq!(events.len(), 2);
        assert!(events.iter().all(|e| e.key == "LOC-1"));
        assert_eq!(events[0].to, Status::Queued);
        assert_eq!(events[1].to, Status::InProgress);
    }

    #[test]
    fn read_honours_limit_and_returns_most_recent() {
        let _workspace = TempWorkspace::new("limit");
        let vault = _workspace.path();
        for i in 0..5 {
            append(vault, &event(&format!("LOC-{i}"), Status::Queued));
        }

        let events = read(vault, None, 2);

        assert_eq!(events.len(), 2);
        assert_eq!(events[0].key, "LOC-3");
        assert_eq!(events[1].key, "LOC-4");
    }

    #[test]
    fn malformed_line_is_skipped_not_fatal() {
        let _workspace = TempWorkspace::new("malformed");
        let vault = _workspace.path();
        append(vault, &event("LOC-1", Status::Queued));

        let file = events_path(vault);
        let mut f = fs::OpenOptions::new().append(true).open(&file).unwrap();
        writeln!(f, "not valid json").unwrap();

        append(vault, &event("LOC-2", Status::Queued));

        let events = read(vault, None, 10);

        assert_eq!(events.len(), 2);
        assert_eq!(events[0].key, "LOC-1");
        assert_eq!(events[1].key, "LOC-2");
    }

    /// Proves the bound is real: a journal far bigger than one chunk (and
    /// bigger than a naive full-file parse would want to repeat for every
    /// call) still yields exactly the last N events, in order — written
    /// directly with one `fs::write` rather than 20 000 calls to `append`.
    #[test]
    fn read_of_a_large_journal_is_bounded() {
        let _workspace = TempWorkspace::new("large");
        let vault = _workspace.path();
        fs::create_dir_all(vault.join("TODO").join(".state")).unwrap();

        const N: usize = 20_000;
        let mut journal = String::new();
        for i in 0..N {
            let e = event(&format!("LOC-{i}"), Status::Queued);
            journal.push_str(&serde_json::to_string(&e).unwrap());
            journal.push('\n');
        }
        fs::write(events_path(vault), journal).unwrap();

        let events = read(vault, None, 5);

        let keys: Vec<&str> = events.iter().map(|e| e.key.as_str()).collect();
        assert_eq!(
            keys,
            vec![
                "LOC-19995",
                "LOC-19996",
                "LOC-19997",
                "LOC-19998",
                "LOC-19999"
            ]
        );
    }

    /// The bug a naive chunked reader ships with: a record whose bytes span
    /// two chunks gets truncated or dropped instead of reconstructed. Every
    /// line here is forced to the same fixed, odd byte length, which (since
    /// the chunk size is a power of two) guarantees every chunk boundary the
    /// backward scan makes falls strictly inside some line rather than
    /// neatly between two — and a handful of `TARGET`-keyed lines are placed
    /// exactly on those boundaries, at different depths in the file.
    #[test]
    fn read_by_key_reaches_back_across_chunks() {
        let _workspace = TempWorkspace::new("chunked-key");
        let vault = _workspace.path();
        fs::create_dir_all(vault.join("TODO").join(".state")).unwrap();

        const CHUNK_SIZE: u64 = 64 * 1024;
        const N: usize = 3_000;

        // Every field but `reason` is fixed-width (zero-padded time, a
        // fixed-width key, a zero-padded index in `session_id`), so padding
        // `reason` by a single constant amount makes every line the same
        // byte length, independent of `i`.
        fn raw_line(i: usize, key: &str, pad_len: usize) -> String {
            let secs = i % 60;
            let mins = (i / 60) % 60;
            let hours = (i / 3600) % 24;
            format!(
                "{{\"ts\":\"2024-01-01T{hours:02}:{mins:02}:{secs:02}Z\",\"key\":\"{key}\",\
                 \"from\":\"todo\",\"to\":\"queued\",\"actor\":\"cli\",\"reason\":\"{pad}\",\
                 \"session_id\":\"{i:04}\"}}",
                pad = "x".repeat(pad_len)
            )
        }

        // Find the minimal padding that makes the (odd/even-agnostic) base
        // line an ODD number of bytes once its trailing '\n' is counted —
        // an odd length can never evenly divide the power-of-two chunk
        // size, so a boundary can never land exactly between two lines.
        let base_len = raw_line(0, "OTHER1", 0).len();
        let mut pad_len = 0usize;
        while (base_len + pad_len + 1).is_multiple_of(2) {
            pad_len += 1;
        }
        let line_len = (base_len + pad_len + 1) as u64;

        let total_len = line_len * N as u64;
        assert!(
            total_len > CHUNK_SIZE * 3,
            "test setup: journal must span several chunks"
        );

        // Every boundary the backward scan will cut at, and the index of
        // the line each one falls inside.
        let mut boundary_indices = Vec::new();
        let mut k = 1u64;
        while k * CHUNK_SIZE < total_len {
            let boundary = total_len - k * CHUNK_SIZE;
            assert_ne!(
                boundary % line_len,
                0,
                "test setup: boundary must straddle a line, not fall between two"
            );
            boundary_indices.push((boundary / line_len) as usize);
            k += 1;
        }
        assert!(
            boundary_indices.len() >= 3,
            "test setup: need several chunk boundaries at different depths"
        );

        let mut target_indices = vec![
            boundary_indices[0],
            boundary_indices[boundary_indices.len() / 2],
            *boundary_indices.last().unwrap(),
        ];
        target_indices.sort_unstable();
        target_indices.dedup();

        let mut journal = String::with_capacity(line_len as usize * N);
        for i in 0..N {
            let key = if target_indices.contains(&i) {
                "TARGET"
            } else {
                "OTHER1"
            };
            journal.push_str(&raw_line(i, key, pad_len));
            journal.push('\n');
        }
        fs::write(events_path(vault), journal).unwrap();

        let events = read(vault, Some("TARGET"), target_indices.len());

        assert_eq!(events.len(), target_indices.len());
        for (event, &idx) in events.iter().zip(target_indices.iter()) {
            assert_eq!(event.key, "TARGET");
            assert_eq!(
                event.session_id.as_deref(),
                Some(format!("{idx:04}").as_str())
            );
        }
    }
}
