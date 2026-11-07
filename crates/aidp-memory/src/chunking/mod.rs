//! Cutting files into chunks: Markdown by headings, C# by type and method.

pub mod csharp;
pub mod markdown;

/// A run of consecutive lines cut by [`pack_lines`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Piece {
    pub line_start: usize,
    pub line_end: usize,
    pub text: String,
}

/// Packs numbered lines into pieces of at most `max` characters.
///
/// The next piece starts with up to `overlap` characters of trailing lines of
/// the previous one. A line longer than `max` is cut on a character boundary.
pub fn pack_lines(lines: &[(usize, &str)], max: usize, overlap: usize) -> Vec<Piece> {
    let mut pieces: Vec<Piece> = Vec::new();
    let mut buffer: Vec<(usize, String)> = Vec::new();
    let mut buffer_chars = 0usize;
    let mut fresh = 0usize;
    for &(number, line) in lines {
        for part in split_long_line(line, max) {
            let part_chars = part.chars().count() + 1;
            if buffer_chars + part_chars > max && fresh > 0 {
                emit(&buffer, &mut pieces);
                buffer = overlap_tail(&buffer, overlap);
                buffer_chars = buffer
                    .iter()
                    .map(|(_, text)| text.chars().count() + 1)
                    .sum();
                fresh = 0;
            }
            buffer_chars += part_chars;
            if !part.trim().is_empty() {
                fresh += 1;
            }
            buffer.push((number, part));
        }
    }
    if fresh > 0 {
        emit(&buffer, &mut pieces);
    }
    pieces
}

fn split_long_line(line: &str, max: usize) -> Vec<String> {
    if line.chars().count() <= max {
        return vec![line.to_string()];
    }
    let characters: Vec<char> = line.chars().collect();
    characters
        .chunks(max)
        .map(|part| part.iter().collect())
        .collect()
}

fn overlap_tail(buffer: &[(usize, String)], overlap: usize) -> Vec<(usize, String)> {
    let mut kept: Vec<(usize, String)> = Vec::new();
    let mut total = 0usize;
    for (number, text) in buffer.iter().rev() {
        let size = text.chars().count() + 1;
        if total + size > overlap || kept.len() + 1 >= buffer.len() {
            break;
        }
        total += size;
        kept.push((*number, text.clone()));
    }
    kept.reverse();
    kept
}

fn emit(buffer: &[(usize, String)], pieces: &mut Vec<Piece>) {
    let first = buffer.iter().position(|(_, text)| !text.trim().is_empty());
    let last = buffer.iter().rposition(|(_, text)| !text.trim().is_empty());
    let (Some(first), Some(last)) = (first, last) else {
        return;
    };
    let text = buffer[first..=last]
        .iter()
        .map(|(_, text)| text.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    pieces.push(Piece {
        line_start: buffer[first].0,
        line_end: buffer[last].0,
        text,
    });
}

#[cfg(test)]
#[path = "../../tests/unit/chunking_tests.rs"]
mod tests;
