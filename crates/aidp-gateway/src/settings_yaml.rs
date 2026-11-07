//! Line-level edits of a block-style YAML file that keep its comments, blank lines and key order.
//! Every function answers `None` (or leaves the text alone) when the shape is out of reach, and
//! the caller then rewrites the file from the parsed document.

use serde_yaml::Value;

const COMMENT_MARK: char = '#';
const SEQUENCE_MARK: &str = "-";
const SEQUENCE_ITEM_PREFIX: &str = "- ";
const NON_KEY_STARTS: [char; 4] = ['#', '"', '\u{27}', '?'];
const QUOTE_MARKS: [char; 2] = ['"', '\u{27}'];
const CHILD_INDENT: usize = 2;
const WINDOWS_NEWLINE: &str = "\r\n";
const UNIX_NEWLINE: &str = "\n";
const MULTILINE_MARKS: [char; 2] = ['|', '>'];
const SIZE_SUFFIXES: [char; 6] = ['k', 'K', 'm', 'M', 'g', 'G'];

/// One leaf to set (`Some`) or remove (`None`), addressed by its key path.
#[derive(Debug, Clone, PartialEq)]
pub struct LeafEdit {
    pub path: Vec<String>,
    pub value: Option<Value>,
}

fn indent_of(line: &str) -> usize {
    line.len() - line.trim_start_matches(' ').len()
}

fn is_content(line: &str) -> bool {
    let trimmed = line.trim();
    !trimmed.is_empty() && !trimmed.starts_with(COMMENT_MARK)
}

fn key_of(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    if trimmed.starts_with(NON_KEY_STARTS) || trimmed.starts_with(SEQUENCE_ITEM_PREFIX) {
        return None;
    }
    let colon = trimmed.find(':')?;
    let rest = &trimmed[colon + 1..];
    (rest.is_empty() || rest.starts_with(' ')).then(|| &trimmed[..colon])
}

fn is_sequence_item(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed == SEQUENCE_MARK || trimmed.starts_with(SEQUENCE_ITEM_PREFIX)
}

/// Splits what follows `key:` into the value text and the comment (with its offset in `rest`).
fn split_value(rest: &str) -> (&str, Option<(usize, &str)>) {
    let mut quote: Option<char> = None;
    let mut previous = ' ';
    for (position, character) in rest.char_indices() {
        match quote {
            Some(open) if character == open => quote = None,
            Some(_) => {}
            None if QUOTE_MARKS.contains(&character) => quote = Some(character),
            None if character == COMMENT_MARK && previous.is_whitespace() => {
                return (rest[..position].trim(), Some((position, &rest[position..])));
            }
            None => {}
        }
        previous = character;
    }
    (rest.trim(), None)
}

/// Index just after the last content line of the block that starts at `key_index`.
fn block_end(lines: &[String], key_index: usize, key_indent: usize) -> usize {
    let mut end = key_index + 1;
    for (offset, line) in lines.iter().enumerate().skip(key_index + 1) {
        if !is_content(line) {
            continue;
        }
        let indent = indent_of(line);
        if indent > key_indent || (indent == key_indent && is_sequence_item(line)) {
            end = offset + 1;
        } else {
            break;
        }
    }
    end
}

fn first_content_indent(lines: &[String], start: usize, end: usize) -> Option<usize> {
    lines[start..end]
        .iter()
        .find(|line| is_content(line))
        .map(|line| indent_of(line))
}

fn find_child(
    lines: &[String],
    start: usize,
    end: usize,
    indent: usize,
    key: &str,
) -> Option<usize> {
    (start..end).find(|&index| {
        is_content(&lines[index])
            && indent_of(&lines[index]) == indent
            && key_of(&lines[index]) == Some(key)
    })
}

fn is_plain_amount(text: &str) -> bool {
    let digits = text.strip_suffix(SIZE_SUFFIXES).unwrap_or(text);
    let mut parts = digits.split('.');
    let whole = parts.next().unwrap_or_default();
    let fraction = parts.next();
    let well_formed = !whole.is_empty()
        && whole.chars().all(|c| c.is_ascii_digit())
        && fraction.is_none_or(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit()));
    well_formed && parts.next().is_none()
}

/// A scalar as it is written after `key:`.
pub fn render_scalar(value: &Value) -> String {
    match value {
        Value::String(text) if is_plain_amount(text) => text.clone(),
        Value::String(text) => {
            serde_json::to_string(text).unwrap_or_else(|_| format!("\"{text}\""))
        }
        other => serde_yaml::to_string(other)
            .map(|text| text.trim().to_string())
            .unwrap_or_default(),
    }
}

fn replace_value(line: &str, value: &Value) -> Option<String> {
    let indent = indent_of(line);
    let trimmed = line.trim_start();
    let colon = trimmed.find(':')?;
    let key = &trimmed[..colon];
    let rest = &trimmed[colon + 1..];
    let (old, comment) = split_value(rest);
    if old.is_empty() || old.starts_with(MULTILINE_MARKS) {
        return None;
    }
    let mut result = format!("{}{key}: {}", " ".repeat(indent), render_scalar(value));
    if let Some((position, text)) = comment {
        let original_column = indent + key.len() + 1 + position;
        let gap = original_column
            .saturating_sub(result.chars().count())
            .max(1);
        result.push_str(&" ".repeat(gap));
        result.push_str(text);
    }
    Some(result)
}

fn set_leaf(lines: &mut Vec<String>, path: &[String], value: Option<&Value>) -> Option<()> {
    let (mut start, mut end) = (0, lines.len());
    let mut parent_indent: Option<usize> = None;
    for (depth, key) in path.iter().enumerate() {
        let last = depth + 1 == path.len();
        let child_indent = first_content_indent(lines, start, end)
            .unwrap_or_else(|| parent_indent.map_or(0, |indent| indent + CHILD_INDENT));
        let Some(index) = find_child(lines, start, end, child_indent, key) else {
            let Some(value) = value else {
                return Some(());
            };
            let mut added = Vec::new();
            for (step, name) in path[depth..].iter().enumerate() {
                let pad = " ".repeat(child_indent + step * CHILD_INDENT);
                if depth + step + 1 == path.len() {
                    added.push(format!("{pad}{name}: {}", render_scalar(value)));
                } else {
                    added.push(format!("{pad}{name}:"));
                }
            }
            lines.splice(end..end, added);
            return Some(());
        };
        let block = block_end(lines, index, child_indent);
        if last {
            match value {
                Some(value) => lines[index] = replace_value(&lines[index], value)?,
                None if block == index + 1 => {
                    lines.remove(index);
                }
                None => return None,
            }
            return Some(());
        }
        let colon = lines[index].find(':')?;
        if !split_value(&lines[index][colon + 1..]).0.is_empty() {
            return None;
        }
        (start, end, parent_indent) = (index + 1, block, Some(child_indent));
    }
    Some(())
}

fn split_lines(text: &str) -> (Vec<String>, &'static str) {
    let newline = if text.contains(WINDOWS_NEWLINE) {
        WINDOWS_NEWLINE
    } else {
        UNIX_NEWLINE
    };
    (text.lines().map(str::to_string).collect(), newline)
}

fn join_lines(lines: &[String], newline: &str) -> String {
    let mut text = lines.join(newline);
    if !lines.is_empty() {
        text.push_str(newline);
    }
    text
}

/// The text with every leaf edit applied in place; `None` when one of them needs a rewrite.
pub fn patch_leaves(text: &str, edits: &[LeafEdit]) -> Option<String> {
    if text.contains('\t') {
        return None;
    }
    let (mut lines, newline) = split_lines(text);
    for edit in edits {
        set_leaf(&mut lines, &edit.path, edit.value.as_ref())?;
    }
    Some(join_lines(&lines, newline))
}

/// The text with the top-level block `key` replaced by `rendered` (a whole `key:` block), appended
/// when absent, or dropped when `rendered` is `None`.
pub fn replace_section(text: &str, key: &str, rendered: Option<&str>) -> String {
    let (mut lines, newline) = split_lines(text);
    let added: Vec<String> = rendered
        .map(|block| block.lines().map(str::to_string).collect())
        .unwrap_or_default();
    match find_child(&lines, 0, lines.len(), 0, key) {
        Some(index) => {
            let end = block_end(&lines, index, 0);
            lines.splice(index..end, added);
        }
        None if !added.is_empty() => {
            if lines.last().is_some_and(|line| !line.trim().is_empty()) {
                lines.push(String::new());
            }
            lines.extend(added);
        }
        None => {}
    }
    join_lines(&lines, newline)
}

/// Sets (`Some`) or removes (`None`) the leaf at `path` in a parsed document.
pub fn apply_to_value(document: &mut Value, path: &[String], value: Option<&Value>) {
    let Some((key, parents)) = path.split_last() else {
        return;
    };
    let mut current = document;
    for parent in parents {
        if !current.is_mapping() {
            *current = Value::Mapping(Default::default());
        }
        let Some(mapping) = current.as_mapping_mut() else {
            return;
        };
        let name = Value::String(parent.clone());
        if !mapping.get(&name).is_some_and(Value::is_mapping) {
            if value.is_none() {
                return;
            }
            mapping.insert(name.clone(), Value::Mapping(Default::default()));
        }
        let Some(next) = mapping.get_mut(&name) else {
            return;
        };
        current = next;
    }
    if !current.is_mapping() {
        *current = Value::Mapping(Default::default());
    }
    if let Some(mapping) = current.as_mapping_mut() {
        match value {
            Some(value) => {
                mapping.insert(Value::String(key.clone()), value.clone());
            }
            None => {
                mapping.remove(Value::String(key.clone()));
            }
        }
    }
}

#[cfg(test)]
#[path = "../tests/unit/settings_yaml_tests.rs"]
mod tests;
