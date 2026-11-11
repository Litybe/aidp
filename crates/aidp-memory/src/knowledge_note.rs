//! Reads the frontmatter and the title of a knowledge file.

use crate::chunking::markdown::title_of_note;
use crate::models::constants::knowledge::GLOSSARY_NOTE_TYPE;
use crate::models::constants::notes::FRONTMATTER_FENCE;
use crate::models::enums::NoteStatus;
use crate::models::knowledge_note::KnowledgeNote;
use crate::models::note_frontmatter::NoteFrontmatter;
use crate::models::raw_chunk::RawChunk;

const TITLE_JOINER: &str = " > ";

/// The frontmatter YAML and the body of a file; `None` without a closed frontmatter.
pub fn split_frontmatter(content: &str) -> Option<(String, String)> {
    let normalized = content.replace("\r\n", "\n");
    let fence = format!("{FRONTMATTER_FENCE}\n");
    let rest = normalized
        .trim_start_matches('\u{feff}')
        .strip_prefix(&fence)?;
    let (yaml, body) = rest.split_once(&format!("\n{fence}")).or_else(|| {
        rest.strip_suffix(&format!("\n{FRONTMATTER_FENCE}"))
            .map(|yaml| (yaml, ""))
    })?;
    Some((yaml.to_string(), body.to_string()))
}

/// Parses a knowledge file; a missing or invalid frontmatter reads as an empty one.
pub fn parse_knowledge(content: &str, fallback_title: &str) -> KnowledgeNote {
    let (yaml, body) =
        split_frontmatter(content).unwrap_or_else(|| (String::new(), content.to_string()));
    let front: NoteFrontmatter = serde_yaml::from_str(&yaml).unwrap_or_default();
    let title = front
        .title
        .clone()
        .filter(|title| !title.trim().is_empty())
        .or_else(|| title_of_note(&body))
        .unwrap_or_else(|| fallback_title.to_string());
    KnowledgeNote { front, title }
}

impl KnowledgeNote {
    /// Whether the file feeds the index: not the glossary, not proposed or superseded.
    pub fn is_indexable(&self) -> bool {
        if self
            .front
            .note_type
            .as_deref()
            .is_some_and(|kind| kind.trim().eq_ignore_ascii_case(GLOSSARY_NOTE_TYPE))
        {
            return false;
        }
        self.front
            .status
            .as_deref()
            .and_then(|status| NoteStatus::parse(status).ok())
            .is_none_or(NoteStatus::is_retrievable)
    }

    /// Puts the note title first in every chunk title.
    pub fn title_chunks(&self, chunks: &mut [RawChunk], fallback_title: &str) {
        for chunk in chunks {
            if chunk.title == fallback_title {
                chunk.title = self.title.clone();
            } else if !chunk.title.starts_with(&self.title) {
                chunk.title = format!("{}{TITLE_JOINER}{}", self.title, chunk.title);
            }
        }
    }
}

#[cfg(test)]
#[path = "../tests/unit/knowledge_note_tests.rs"]
mod tests;
