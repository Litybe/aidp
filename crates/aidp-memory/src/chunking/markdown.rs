//! Markdown chunker: one section per heading, long sections cut with a light overlap.

use super::pack_lines;
use crate::models::constants::limits::{MARKDOWN_MAX_CHARS, MARKDOWN_OVERLAP_CHARS};
use crate::models::raw_chunk::RawChunk;

const FRONTMATTER_FENCE: &str = "---";
const FRONTMATTER_MAX_LINES: usize = 200;
const CODE_FENCES: [&str; 2] = ["```", "~~~"];
const HEADING_MARK: char = '#';
const MAX_HEADING_LEVEL: usize = 6;
const TITLE_JOINER: &str = " > ";
const KIND_SECTION: &str = "section";

/// Cuts a Markdown or plain text file; `fallback_title` names text before the first heading.
pub fn chunk_markdown(text: &str, fallback_title: &str) -> Vec<RawChunk> {
    let lines: Vec<&str> = text.lines().collect();
    let first_line = frontmatter_end(&lines);
    let mut chunks: Vec<RawChunk> = Vec::new();
    let mut headings: Vec<(usize, String)> = Vec::new();
    let mut section: Vec<(usize, &str)> = Vec::new();
    let mut title = fallback_title.to_string();
    let mut in_fence = false;
    for (index, line) in lines.iter().enumerate().skip(first_line) {
        let trimmed = line.trim_start();
        if CODE_FENCES.iter().any(|fence| trimmed.starts_with(fence)) {
            in_fence = !in_fence;
        }
        let heading = if in_fence { None } else { parse_heading(line) };
        if let Some((level, name)) = heading {
            flush_section(&section, &title, &mut chunks);
            section.clear();
            while headings.last().is_some_and(|(open, _)| *open >= level) {
                headings.pop();
            }
            headings.push((level, name));
            title = headings
                .iter()
                .map(|(_, name)| name.as_str())
                .collect::<Vec<_>>()
                .join(TITLE_JOINER);
        }
        section.push((index + 1, line));
    }
    flush_section(&section, &title, &mut chunks);
    chunks
}

/// The text of the first level-one heading of a body, outside code fences.
pub fn title_of_note(body: &str) -> Option<String> {
    let mut in_fence = false;
    for line in body.lines() {
        if CODE_FENCES
            .iter()
            .any(|fence| line.trim_start().starts_with(fence))
        {
            in_fence = !in_fence;
        }
        if in_fence {
            continue;
        }
        if let Some((1, name)) = parse_heading(line) {
            return Some(name);
        }
    }
    None
}

fn frontmatter_end(lines: &[&str]) -> usize {
    if lines.first().map(|line| line.trim()) != Some(FRONTMATTER_FENCE) {
        return 0;
    }
    lines
        .iter()
        .enumerate()
        .skip(1)
        .take(FRONTMATTER_MAX_LINES)
        .find(|(_, line)| line.trim() == FRONTMATTER_FENCE)
        .map_or(0, |(index, _)| index + 1)
}

fn parse_heading(line: &str) -> Option<(usize, String)> {
    let level = line
        .chars()
        .take_while(|&character| character == HEADING_MARK)
        .count();
    if level == 0 || level > MAX_HEADING_LEVEL {
        return None;
    }
    let rest = &line[level..];
    if !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let name = rest.trim().trim_end_matches(HEADING_MARK).trim();
    if name.is_empty() {
        None
    } else {
        Some((level, name.to_string()))
    }
}

fn flush_section(section: &[(usize, &str)], title: &str, chunks: &mut Vec<RawChunk>) {
    for piece in pack_lines(section, MARKDOWN_MAX_CHARS, MARKDOWN_OVERLAP_CHARS) {
        let heading_only = piece
            .text
            .lines()
            .filter(|line| !line.trim().is_empty())
            .all(|line| line.starts_with(HEADING_MARK));
        if heading_only {
            continue;
        }
        chunks.push(RawChunk {
            title: title.to_string(),
            text: piece.text,
            line_start: piece.line_start,
            line_end: piece.line_end,
            kind: KIND_SECTION,
            symbol: None,
        });
    }
}

#[cfg(test)]
#[path = "../../tests/unit/markdown_tests.rs"]
mod tests;
