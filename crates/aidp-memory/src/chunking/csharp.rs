//! Light C# chunker: a brace and signature scanner, no syntax tree.
//!
//! Each method is one chunk, each type keeps its fields and properties in
//! "members" chunks, and a doc comment or attribute stays with its member.

use super::pack_lines;
use crate::models::constants::limits::CODE_MAX_CHARS;
use crate::models::raw_chunk::RawChunk;
use regex::Regex;
use std::sync::LazyLock;

const KIND_METHOD: &str = "method";
const KIND_TYPE: &str = "type";
const KIND_MEMBERS: &str = "members";
const KIND_TEXT: &str = "text";
const MEMBERS_SUFFIX: &str = " (members)";
const NAMESPACE_KEYWORD: &str = "namespace ";
const QUALIFIER: char = '.';
const CONTROL_WORDS: [&str; 15] = [
    "if", "else", "for", "foreach", "while", "switch", "using", "lock", "catch", "fixed", "do",
    "try", "finally", "unsafe", "checked",
];
const TRIVIAL_LINES: [&str; 3] = ["", "{", "}"];

static TYPE_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(?:(?:record\s+)?(?:class|struct|interface|enum)|record)\s+([A-Za-z_]\w*)")
        .expect("type pattern")
});
static ATTRIBUTE_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*\[[^\]]*\]").expect("attribute pattern"));
static IDENTIFIER_PATTERN: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"([A-Za-z_]\w*)\s*(?:<[^<>()]*>)?\s*$").expect("identifier"));

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Code,
    BlockComment,
    Text,
    Verbatim,
    Character,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FrameKind {
    Namespace,
    Type,
    Method,
    Initializer,
    Other,
}

struct Frame {
    kind: FrameKind,
    name: String,
    start_line: usize,
    member_start: Option<usize>,
    spans: Vec<(usize, usize)>,
}

struct Parser<'a> {
    lines: Vec<&'a str>,
    stack: Vec<Frame>,
    header: String,
    header_start: Option<usize>,
    chunks: Vec<RawChunk>,
}

/// Cuts a C# source file; `file_stem` titles text found outside any type.
pub fn chunk_csharp(source: &str, file_stem: &str) -> Vec<RawChunk> {
    let mut parser = Parser {
        lines: source.lines().collect(),
        stack: Vec::new(),
        header: String::new(),
        header_start: None,
        chunks: Vec::new(),
    };
    parser.scan();
    if parser.chunks.is_empty() {
        let numbered: Vec<(usize, &str)> = parser
            .lines
            .iter()
            .enumerate()
            .map(|(index, line)| (index + 1, *line))
            .collect();
        for piece in pack_lines(&numbered, CODE_MAX_CHARS, 0) {
            parser.chunks.push(RawChunk {
                title: file_stem.to_string(),
                text: piece.text,
                line_start: piece.line_start,
                line_end: piece.line_end,
                kind: KIND_TEXT,
                symbol: None,
            });
        }
    }
    parser.chunks
}

impl Parser<'_> {
    fn scan(&mut self) {
        let mut mode = Mode::Code;
        for index in 0..self.lines.len() {
            let number = index + 1;
            let line = self.lines[index];
            if mode == Mode::Code && line.trim_start().starts_with('#') {
                continue;
            }
            let characters: Vec<char> = line.chars().collect();
            let mut position = 0usize;
            while position < characters.len() {
                let current = characters[position];
                let next = characters.get(position + 1).copied();
                match mode {
                    Mode::BlockComment => {
                        if current == '*' && next == Some('/') {
                            mode = Mode::Code;
                            position += 1;
                        }
                    }
                    Mode::Text => {
                        if current == '\\' {
                            position += 1;
                        } else if current == '"' {
                            mode = Mode::Code;
                        }
                    }
                    Mode::Verbatim => {
                        if current == '"' {
                            if next == Some('"') {
                                position += 1;
                            } else {
                                mode = Mode::Code;
                            }
                        }
                    }
                    Mode::Character => {
                        if current == '\\' {
                            position += 1;
                        } else if current == '\'' {
                            mode = Mode::Code;
                        }
                    }
                    Mode::Code => {
                        if current == '/' && next == Some('/') {
                            self.touch(number);
                            break;
                        }
                        if current == '/' && next == Some('*') {
                            self.touch(number);
                            mode = Mode::BlockComment;
                            position += 1;
                        } else if current == '"' {
                            self.touch(number);
                            let verbatim = (position > 0 && characters[position - 1] == '@')
                                || (position > 1
                                    && characters[position - 1] == '$'
                                    && characters[position - 2] == '@');
                            mode = if verbatim { Mode::Verbatim } else { Mode::Text };
                            self.push_header('"');
                        } else if current == '\'' {
                            self.touch(number);
                            mode = Mode::Character;
                            self.push_header('\'');
                        } else if current == '{' {
                            self.open(number);
                        } else if current == '}' {
                            self.close(number);
                        } else if current == ';' {
                            self.semicolon(number);
                        } else if current.is_whitespace() {
                            if !self.header.is_empty() {
                                self.push_header(' ');
                            }
                        } else {
                            self.touch(number);
                            self.push_header(current);
                        }
                    }
                }
                position += 1;
            }
            if matches!(mode, Mode::Text | Mode::Character) {
                mode = Mode::Code;
            }
        }
    }

    fn touch(&mut self, number: usize) {
        if self.header_start.is_none() {
            self.header_start = Some(number);
        }
    }

    fn collecting(&self) -> bool {
        self.stack
            .last()
            .is_none_or(|frame| matches!(frame.kind, FrameKind::Namespace | FrameKind::Type))
    }

    fn in_type(&self) -> bool {
        self.stack
            .last()
            .is_some_and(|frame| frame.kind == FrameKind::Type)
    }

    fn push_header(&mut self, character: char) {
        if self.collecting() {
            self.header.push(character);
        }
    }

    fn open(&mut self, number: usize) {
        let (kind, name) = if self.collecting() {
            classify(&self.header, self.in_type())
        } else {
            (FrameKind::Other, String::new())
        };
        let name = match kind {
            FrameKind::Type => self.qualify(&name),
            _ => name,
        };
        self.stack.push(Frame {
            kind,
            name,
            start_line: self.header_start.unwrap_or(number),
            member_start: self.header_start,
            spans: Vec::new(),
        });
        self.header.clear();
        self.header_start = None;
    }

    fn qualify(&self, name: &str) -> String {
        match self
            .stack
            .iter()
            .rev()
            .find(|frame| frame.kind == FrameKind::Type)
        {
            Some(owner) => format!("{}{QUALIFIER}{name}", owner.name),
            None => name.to_string(),
        }
    }

    fn close(&mut self, number: usize) {
        let Some(frame) = self.stack.pop() else {
            return;
        };
        match frame.kind {
            FrameKind::Method => {
                let owner = self.owner_name();
                let title = format!("{owner}{QUALIFIER}{}", frame.name);
                self.emit_range(frame.start_line, number, &title, KIND_METHOD, &title);
                self.add_span(frame.start_line, number);
            }
            FrameKind::Type => {
                self.emit_type(&frame, number);
                self.add_span(frame.start_line, number);
            }
            FrameKind::Initializer => {
                self.header_start = frame.member_start;
                self.header = "=".to_string();
                return;
            }
            FrameKind::Namespace | FrameKind::Other => {}
        }
        self.header.clear();
        self.header_start = None;
    }

    fn semicolon(&mut self, number: usize) {
        if self.in_type() && self.header.contains('(') && self.header.contains("=>") {
            let (kind, name) = classify(&self.header, true);
            if kind == FrameKind::Method {
                let start = self.header_start.unwrap_or(number);
                let title = format!("{}{QUALIFIER}{name}", self.owner_name());
                self.emit_range(start, number, &title, KIND_METHOD, &title);
                self.add_span(start, number);
            }
        }
        self.header.clear();
        self.header_start = None;
    }

    fn owner_name(&self) -> String {
        self.stack
            .iter()
            .rev()
            .find(|frame| frame.kind == FrameKind::Type)
            .map(|frame| frame.name.clone())
            .unwrap_or_default()
    }

    fn add_span(&mut self, start: usize, end: usize) {
        if let Some(parent) = self.stack.last_mut() {
            if parent.kind == FrameKind::Type {
                parent.spans.push((start, end));
            }
        }
    }

    fn emit_range(
        &mut self,
        start: usize,
        end: usize,
        title: &str,
        kind: &'static str,
        symbol: &str,
    ) {
        let end = end.min(self.lines.len());
        if start == 0 || start > end {
            return;
        }
        let numbered: Vec<(usize, &str)> = (start..=end).map(|n| (n, self.lines[n - 1])).collect();
        for piece in pack_lines(&numbered, CODE_MAX_CHARS, 0) {
            self.chunks.push(RawChunk {
                title: title.to_string(),
                text: piece.text,
                line_start: piece.line_start,
                line_end: piece.line_end,
                kind,
                symbol: Some(symbol.to_string()),
            });
        }
    }

    fn emit_type(&mut self, frame: &Frame, end: usize) {
        let end = end.min(self.lines.len());
        if frame.start_line == 0 || frame.start_line > end {
            return;
        }
        let mut covered = vec![false; end - frame.start_line + 1];
        for &(span_start, span_end) in &frame.spans {
            for line in span_start.max(frame.start_line)..=span_end.min(end) {
                covered[line - frame.start_line] = true;
            }
        }
        let mut run_start: Option<usize> = None;
        let mut first = true;
        for offset in 0..=covered.len() {
            let is_open = offset < covered.len() && !covered[offset];
            match (is_open, run_start) {
                (true, None) => run_start = Some(offset),
                (false, Some(begin)) => {
                    let start = frame.start_line + begin;
                    let stop = frame.start_line + offset - 1;
                    let trivial =
                        (start..=stop).all(|n| TRIVIAL_LINES.contains(&self.lines[n - 1].trim()));
                    if !trivial {
                        let (title, kind) = if first {
                            (frame.name.clone(), KIND_TYPE)
                        } else {
                            (format!("{}{MEMBERS_SUFFIX}", frame.name), KIND_MEMBERS)
                        };
                        self.emit_range(start, stop, &title, kind, &frame.name);
                        first = false;
                    }
                    run_start = None;
                }
                _ => {}
            }
        }
    }
}

fn classify(header: &str, in_type: bool) -> (FrameKind, String) {
    let stripped = strip_attributes(header);
    let text = stripped.trim();
    if text.starts_with(NAMESPACE_KEYWORD) {
        return (FrameKind::Namespace, String::new());
    }
    let paren = text.find('(');
    if let Some(captures) = TYPE_PATTERN.captures(text) {
        let keyword_start = captures.get(0).map_or(0, |found| found.start());
        if paren.is_none_or(|open| keyword_start < open) {
            return (FrameKind::Type, captures[1].to_string());
        }
    }
    if !in_type {
        return (FrameKind::Other, String::new());
    }
    if let Some(equal) = first_assignment(text) {
        if paren.is_none_or(|open| equal < open) {
            return (FrameKind::Initializer, String::new());
        }
    }
    if let Some(open) = paren {
        let first_word = text
            .split(|c: char| !c.is_alphanumeric())
            .next()
            .unwrap_or("");
        if CONTROL_WORDS.contains(&first_word) {
            return (FrameKind::Other, String::new());
        }
        if let Some(captures) = IDENTIFIER_PATTERN.captures(text[..open].trim_end()) {
            return (FrameKind::Method, captures[1].to_string());
        }
    }
    (FrameKind::Other, String::new())
}

fn strip_attributes(header: &str) -> String {
    let mut current = header.to_string();
    while let Some(found) = ATTRIBUTE_PATTERN.find(&current) {
        current = current[found.end()..].to_string();
    }
    current
}

fn first_assignment(text: &str) -> Option<usize> {
    let bytes = text.as_bytes();
    (0..bytes.len()).find(|&index| {
        bytes[index] == b'='
            && bytes
                .get(index + 1)
                .is_none_or(|next| *next != b'>' && *next != b'=')
            && (index == 0 || !matches!(bytes[index - 1], b'=' | b'!' | b'<' | b'>'))
    })
}

#[cfg(test)]
#[path = "../../tests/unit/csharp_tests.rs"]
mod tests;
