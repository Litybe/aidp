//! Reads a Claude Code transcript into a journal, and renders that journal as a note.

use crate::error::Result;
use crate::models::constants::sessions::{
    COMMIT_PATTERN, EDIT_TOOLS, HARNESS_MESSAGE_PREFIX, JOURNAL_HEADING_CHARS, JOURNAL_MAX_COMMITS,
    JOURNAL_MAX_FILES, JOURNAL_MAX_PROMPTS, JOURNAL_PROMPT_CHARS, JOURNAL_REPLY_CHARS,
    JOURNAL_SESSION_CHARS, PIPELINE_PROMPT_PREFIX, SHELL_TOOLS,
};
use crate::models::session_journal::SessionJournal;
use regex::Regex;
use serde_json::Value;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

const TYPE_FIELD: &str = "type";
const TYPE_USER: &str = "user";
const TYPE_ASSISTANT: &str = "assistant";
const TYPE_TEXT: &str = "text";
const TYPE_TOOL_USE: &str = "tool_use";
const LINE_MARKERS: [&str; 2] = ["\"type\":\"user\"", "\"type\":\"assistant\""];
const ELLIPSIS: &str = "...";
const FILE_EXTENSION: &str = "md";
const MONTH_CHARS: usize = 7;
const DAY_CHARS: usize = 10;

static COMMIT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(COMMIT_PATTERN).expect("commit pattern"));

impl SessionJournal {
    /// True for a session of the user that asked for or changed something lasting.
    pub fn worth_keeping(&self) -> bool {
        let from_pipeline = self
            .prompts
            .first()
            .is_some_and(|prompt| prompt.starts_with(PIPELINE_PROMPT_PREFIX));
        !from_pipeline
            && self.started.is_some()
            && (self.prompt_count >= 2 || !self.files.is_empty() || !self.commits.is_empty())
    }

    /// `YYYY-MM/YYYY-MM-DD-<session>.md`, stable for the whole session.
    pub fn relative_path(&self) -> PathBuf {
        let started = self.started.as_deref().unwrap_or_default();
        let month: String = started.chars().take(MONTH_CHARS).collect();
        let day: String = started.chars().take(DAY_CHARS).collect();
        let session: String = self
            .session_id
            .chars()
            .take(JOURNAL_SESSION_CHARS)
            .collect();
        Path::new(&month).join(format!("{day}-{session}.{FILE_EXTENSION}"))
    }

    /// The Markdown note of the session.
    pub fn render(&self, project: &str) -> String {
        let mut note = format!(
            "---\nsession: {}\nproject: {project}\nbranch: {}\nstarted: {}\nended: {}\nprompts: {}\n---\n\n# Session {}\n",
            self.session_id,
            self.branch.as_deref().unwrap_or_default(),
            self.started.as_deref().unwrap_or_default(),
            self.ended.as_deref().unwrap_or_default(),
            self.prompt_count,
            clipped(self.prompts.first().map_or("", String::as_str), JOURNAL_HEADING_CHARS)
                .replace('\n', " "),
        );
        push_section(&mut note, "Demandes", &self.prompts);
        push_section(&mut note, "Fichiers modifies", &self.files);
        push_section(&mut note, "Commits", &self.commits);
        if let Some(reply) = &self.last_reply {
            note.push_str(&format!("\n## Fin de session\n\n{reply}\n"));
        }
        note
    }
}

fn push_section(note: &mut String, heading: &str, items: &[String]) {
    if items.is_empty() {
        return;
    }
    note.push_str(&format!("\n## {heading}\n\n"));
    for item in items {
        note.push_str(&format!("- {}\n", item.replace('\n', " ")));
    }
}

fn clipped(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let kept: String = text.chars().take(limit).collect();
    format!("{}{ELLIPSIS}", kept.trim_end())
}

/// Builds the journal of a transcript; unreadable lines are skipped.
pub fn read_transcript(path: &Path) -> Result<SessionJournal> {
    let reader = BufReader::new(std::fs::File::open(path)?);
    let mut journal = SessionJournal::default();
    for line in reader.lines().map_while(std::result::Result::ok) {
        if !LINE_MARKERS.iter().any(|marker| line.contains(marker)) {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if entry["isSidechain"] == Value::Bool(true) || entry["isMeta"] == Value::Bool(true) {
            continue;
        }
        match entry[TYPE_FIELD].as_str() {
            Some(TYPE_USER) => read_user(&entry, &mut journal),
            Some(TYPE_ASSISTANT) => read_assistant(&entry, &mut journal),
            _ => {}
        }
    }
    Ok(journal)
}

fn stamp(entry: &Value, journal: &mut SessionJournal) {
    if journal.session_id.is_empty() {
        if let Some(session) = entry["sessionId"].as_str() {
            journal.session_id = session.to_string();
        }
    }
    if let Some(branch) = entry["gitBranch"].as_str().filter(|name| !name.is_empty()) {
        journal.branch = Some(branch.to_string());
    }
    if let Some(time) = entry["timestamp"].as_str() {
        if journal.started.is_none() {
            journal.started = Some(time.to_string());
        }
        journal.ended = Some(time.to_string());
    }
}

fn prompt_text(content: &Value) -> Option<String> {
    let text = match content {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter(|block| block[TYPE_FIELD] == TYPE_TEXT)
            .filter_map(|block| block[TYPE_TEXT].as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => return None,
    };
    let trimmed = text.trim();
    (!trimmed.is_empty() && !trimmed.starts_with(HARNESS_MESSAGE_PREFIX))
        .then(|| trimmed.to_string())
}

fn read_user(entry: &Value, journal: &mut SessionJournal) {
    let Some(text) = prompt_text(&entry["message"]["content"]) else {
        return;
    };
    stamp(entry, journal);
    journal.prompt_count += 1;
    if journal.prompts.len() < JOURNAL_MAX_PROMPTS {
        journal.prompts.push(clipped(&text, JOURNAL_PROMPT_CHARS));
    }
}

fn read_assistant(entry: &Value, journal: &mut SessionJournal) {
    let Some(blocks) = entry["message"]["content"].as_array() else {
        return;
    };
    stamp(entry, journal);
    for block in blocks {
        match block[TYPE_FIELD].as_str() {
            Some(TYPE_TEXT) => {
                if let Some(text) = block[TYPE_TEXT]
                    .as_str()
                    .filter(|text| !text.trim().is_empty())
                {
                    journal.last_reply = Some(clipped(text.trim(), JOURNAL_REPLY_CHARS));
                }
            }
            Some(TYPE_TOOL_USE) => read_tool(block, journal),
            _ => {}
        }
    }
}

fn read_tool(block: &Value, journal: &mut SessionJournal) {
    let name = block["name"].as_str().unwrap_or_default();
    if EDIT_TOOLS.contains(&name) {
        let file = block["input"]["file_path"]
            .as_str()
            .or_else(|| block["input"]["notebook_path"].as_str());
        if let Some(file) = file {
            push_unique(&mut journal.files, file, JOURNAL_MAX_FILES);
        }
    } else if SHELL_TOOLS.contains(&name) {
        if let Some(subject) = block["input"]["command"].as_str().and_then(commit_subject) {
            push_unique(&mut journal.commits, &subject, JOURNAL_MAX_COMMITS);
        }
    }
}

fn commit_subject(command: &str) -> Option<String> {
    let captures = COMMIT.captures(command)?;
    (1..=3)
        .find_map(|group| captures.get(group))
        .map(|subject| subject.as_str().trim().to_string())
}

fn push_unique(list: &mut Vec<String>, item: &str, limit: usize) {
    if list.len() < limit && !list.iter().any(|known| known == item) {
        list.push(item.to_string());
    }
}

#[cfg(test)]
#[path = "../tests/unit/session_journal_tests.rs"]
mod tests;
