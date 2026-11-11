//! Writes and edits the text of knowledge notes: frontmatter fields, the evidence section.

use crate::knowledge_note::split_frontmatter;
use crate::models::constants::knowledge_proposals::{
    EVIDENCE_BULLET, EVIDENCE_HEADING, NOTE_TYPE, SECTION_PREFIX, TITLE_HEADING_PREFIX,
};
use crate::models::constants::notes::FRONTMATTER_FENCE;
use crate::models::note_draft::NoteDraft;

const LINE_BREAK: &str = "\n";
const FIELD_SEPARATOR: char = ':';
const LIST_SEPARATOR: &str = ", ";
const WORD_SEPARATOR: &str = " ";

fn is_plain(item: &str) -> bool {
    !item.is_empty()
        && item
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || "-_./".contains(character))
}

fn list_item(item: &str) -> String {
    if is_plain(item) {
        item.to_string()
    } else {
        serde_json::to_string(item).unwrap_or_else(|_| format!("\"{item}\""))
    }
}

fn list(items: &[String]) -> String {
    let quoted: Vec<String> = items.iter().map(|item| list_item(item)).collect();
    format!("[{}]", quoted.join(LIST_SEPARATOR))
}

/// Renders a note file: frontmatter in the order of the existing notes, title, body, evidence.
pub fn render_note(draft: &NoteDraft) -> String {
    let mut lines = vec![
        FRONTMATTER_FENCE.to_string(),
        format!("type: {NOTE_TYPE}"),
        format!("domain: {}", draft.domain),
        format!("tags: {}", list(&draft.tags)),
        format!("agents: {}", list(&draft.agents)),
        format!("status: {}", draft.status),
        format!("importance: {}", draft.importance),
        format!("updated: {}", draft.updated),
        format!("source: {}", draft.source),
    ];
    lines.extend(
        draft
            .extra
            .iter()
            .map(|(key, value)| format!("{key}{FIELD_SEPARATOR} {value}")),
    );
    lines.push(FRONTMATTER_FENCE.to_string());
    lines.join(LINE_BREAK) + LINE_BREAK + &render_body(&draft.title, &draft.body, &draft.evidence)
}

/// Title, body and evidence section of a note, as they follow the frontmatter.
pub fn render_body(title: &str, body: &str, evidence: &[String]) -> String {
    let mut lines = vec![
        format!("{TITLE_HEADING_PREFIX}{title}"),
        String::new(),
        strip_title(body, title),
    ];
    if !evidence.is_empty() {
        lines.push(String::new());
        lines.push(EVIDENCE_HEADING.to_string());
        lines.extend(evidence.iter().map(|line| evidence_line(line)));
    }
    lines.join(LINE_BREAK) + LINE_BREAK
}

/// The body without a leading `# Title` line, which the file header supplies.
fn strip_title(body: &str, title: &str) -> String {
    let trimmed = body.trim();
    match trimmed.split_once(LINE_BREAK) {
        Some((first, rest)) if first.trim() == format!("{TITLE_HEADING_PREFIX}{title}") => {
            rest.trim().to_string()
        }
        _ => trimmed.to_string(),
    }
}

fn evidence_line(text: &str) -> String {
    let text = &text
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(WORD_SEPARATOR);
    if text.starts_with(EVIDENCE_BULLET) {
        text.to_string()
    } else {
        format!("{EVIDENCE_BULLET}{text}")
    }
}

fn rebuild(yaml_lines: &[String], body: &str) -> String {
    format!(
        "{FRONTMATTER_FENCE}{LINE_BREAK}{}{LINE_BREAK}{FRONTMATTER_FENCE}{LINE_BREAK}{body}",
        yaml_lines.join(LINE_BREAK)
    )
}

fn has_key(line: &str, key: &str) -> bool {
    line.split_once(FIELD_SEPARATOR)
        .is_some_and(|(name, _)| name.trim() == key)
}

/// Sets a frontmatter field to a raw YAML value, adding it when absent; a file without frontmatter is returned unchanged.
pub fn set_field(content: &str, key: &str, value: &str) -> String {
    let Some((yaml, body)) = split_frontmatter(content) else {
        return content.to_string();
    };
    let line = format!("{key}{FIELD_SEPARATOR} {value}");
    let mut lines: Vec<String> = yaml.lines().map(str::to_string).collect();
    match lines.iter_mut().find(|existing| has_key(existing, key)) {
        Some(existing) => *existing = line,
        None => lines.push(line),
    }
    rebuild(&lines, &body)
}

/// Drops the frontmatter lines of the given keys.
pub fn remove_fields(content: &str, keys: &[&str]) -> String {
    let Some((yaml, body)) = split_frontmatter(content) else {
        return content.to_string();
    };
    let lines: Vec<String> = yaml
        .lines()
        .filter(|line| !keys.iter().any(|key| has_key(line, key)))
        .map(str::to_string)
        .collect();
    rebuild(&lines, &body)
}

/// The text after the frontmatter, or the whole content without one.
pub fn body_of(content: &str) -> String {
    split_frontmatter(content).map_or_else(|| content.to_string(), |(_, body)| body)
}

/// The same frontmatter with another body.
pub fn replace_body(content: &str, body: &str) -> String {
    match split_frontmatter(content) {
        Some((yaml, _)) => {
            let lines: Vec<String> = yaml.lines().map(str::to_string).collect();
            rebuild(&lines, body)
        }
        None => body.to_string(),
    }
}

fn evidence_range(lines: &[&str]) -> Option<(usize, usize)> {
    let start = lines
        .iter()
        .rposition(|line| line.trim() == EVIDENCE_HEADING)?;
    let end = lines[start + 1..]
        .iter()
        .position(|line| line.starts_with(SECTION_PREFIX))
        .map_or(lines.len(), |offset| start + 1 + offset);
    Some((start, end))
}

/// The bullets of the `## Evidence` section.
pub fn evidence_of(content: &str) -> Vec<String> {
    let body = body_of(content);
    let lines: Vec<&str> = body.lines().collect();
    evidence_range(&lines).map_or_else(Vec::new, |(start, end)| {
        lines[start + 1..end]
            .iter()
            .filter(|line| !line.trim().is_empty())
            .map(|line| line.trim_end().to_string())
            .collect()
    })
}

/// Whether an evidence bullet already names `source`.
pub fn evidence_has_source(content: &str, source: &str) -> bool {
    evidence_of(content).iter().any(|line| {
        line.trim_start_matches(EVIDENCE_BULLET)
            .split_whitespace()
            .nth(1)
            .is_some_and(|token| token.trim_end_matches(FIELD_SEPARATOR) == source)
    })
}

/// Whether a text holds a line that opens the evidence section.
pub fn has_evidence_heading(text: &str) -> bool {
    text.lines().any(|line| line.trim() == EVIDENCE_HEADING)
}

/// The text without its `## Evidence` section.
pub fn without_evidence(body: &str) -> String {
    let lines: Vec<&str> = body.lines().collect();
    match evidence_range(&lines) {
        Some((start, end)) => {
            let kept: Vec<&str> = lines[..start]
                .iter()
                .chain(lines[end..].iter())
                .copied()
                .collect();
            kept.join(LINE_BREAK).trim_end().to_string() + LINE_BREAK
        }
        None => body.to_string(),
    }
}

/// Adds a bullet at the end of the `## Evidence` section, creating the section when missing.
pub fn append_evidence(content: &str, text: &str) -> String {
    let bullet = evidence_line(text);
    let lines: Vec<&str> = content.lines().collect();
    match evidence_range(&lines) {
        Some((start, end)) => {
            let last = lines[start + 1..end]
                .iter()
                .rposition(|line| !line.trim().is_empty())
                .map_or(start, |offset| start + 1 + offset);
            let mut result: Vec<String> = lines[..=last]
                .iter()
                .map(|line| (*line).to_string())
                .collect();
            result.push(bullet);
            result.extend(lines[last + 1..].iter().map(|line| (*line).to_string()));
            result.join(LINE_BREAK) + LINE_BREAK
        }
        None => format!(
            "{}{LINE_BREAK}{LINE_BREAK}{EVIDENCE_HEADING}{LINE_BREAK}{bullet}{LINE_BREAK}",
            content.trim_end()
        ),
    }
}

#[cfg(test)]
#[path = "../tests/unit/note_text_tests.rs"]
mod tests;
