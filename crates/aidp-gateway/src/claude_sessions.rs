//! Lists the Claude Code transcripts stored under `~/.claude/projects/<slug>/`.

use crate::models::enums::content_part_type::ContentPartType;
use crate::models::enums::stream_message_type::StreamMessageType;
use crate::models::external_session::ExternalSession;
use crate::models::transcript_line::TranscriptLine;
use crate::models::transcript_part::TranscriptPart;
use crate::session_error::SessionError;
use serde::Deserialize;
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

const CLAUDE_DIR: &str = ".claude";
const PROJECTS_DIR: &str = "projects";
const TRANSCRIPT_EXT: &str = "jsonl";
const SLUG_SEPARATOR: char = '-';
const MAX_PROMPT_CHARS: usize = 60;
const MAX_ID_LEN: usize = 64;
const INVALID_ID_MESSAGE: &str = "invalid session id";
const UNKNOWN_SESSION_MESSAGE: &str = "external session not found";

/// Claude Code's directory name for a project path: non-alphanumerics become `-`.
pub fn slug_for(project_path: &str) -> String {
    project_path
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c
            } else {
                SLUG_SEPARATOR
            }
        })
        .collect()
}

fn slug_dir(home: &Path, project_path: &str) -> PathBuf {
    home.join(CLAUDE_DIR)
        .join(PROJECTS_DIR)
        .join(slug_for(project_path))
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID_LEN
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == SLUG_SEPARATOR)
}

/// The text of a part when it deserializes as a text part; anything else is skipped.
fn text_part(part: &serde_json::Value) -> Option<String> {
    let parsed = TranscriptPart::deserialize(part).ok()?;
    match parsed.kind {
        ContentPartType::Text => parsed.text,
        ContentPartType::Other => None,
    }
}

fn text_of(content: &serde_json::Value) -> Option<String> {
    match content {
        serde_json::Value::String(text) => Some(text.clone()),
        serde_json::Value::Array(parts) => parts.iter().find_map(text_part),
        _ => None,
    }
}

fn prompt_of(line: &str) -> Option<String> {
    let parsed = serde_json::from_str::<TranscriptLine>(line).ok()?;
    if parsed.kind != Some(StreamMessageType::User) {
        return None;
    }
    let text = text_of(&parsed.message?.content?)?;
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.chars().take(MAX_PROMPT_CHARS).collect())
}

fn first_prompt(file: &Path) -> String {
    let Ok(handle) = fs::File::open(file) else {
        return String::new();
    };
    BufReader::new(handle)
        .lines()
        .map_while(Result::ok)
        .find_map(|line| prompt_of(&line))
        .unwrap_or_default()
}

fn describe(file: &Path, id: String) -> (SystemTime, ExternalSession) {
    let time = fs::metadata(file)
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let modified = chrono::DateTime::<chrono::Utc>::from(time).to_rfc3339();
    let session = ExternalSession {
        id,
        first_prompt: first_prompt(file),
        modified,
    };
    (time, session)
}

/// Transcripts of one project, newest first. A missing directory yields an empty list.
pub fn list(home: &Path, project_path: &str) -> Vec<ExternalSession> {
    let Ok(entries) = fs::read_dir(slug_dir(home, project_path)) else {
        return Vec::new();
    };
    let mut found: Vec<(SystemTime, ExternalSession)> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|e| e.to_str()) == Some(TRANSCRIPT_EXT))
        .filter_map(|path| {
            let id = path.file_stem()?.to_str()?.to_string();
            Some(describe(&path, id))
        })
        .collect();
    found.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    found.into_iter().map(|(_, session)| session).collect()
}

/// One transcript by id; an invalid or unknown id is `NotFound`.
pub fn find(home: &Path, project_path: &str, id: &str) -> Result<ExternalSession, SessionError> {
    if !valid_id(id) {
        return Err(SessionError::NotFound(INVALID_ID_MESSAGE.to_string()));
    }
    let file = slug_dir(home, project_path).join(format!("{id}.{TRANSCRIPT_EXT}"));
    if !file.is_file() {
        return Err(SessionError::NotFound(UNKNOWN_SESSION_MESSAGE.to_string()));
    }
    Ok(describe(&file, id.to_string()).1)
}

#[cfg(test)]
#[path = "../tests/unit/claude_sessions_tests.rs"]
mod tests;
