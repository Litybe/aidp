//! Splits a markdown comment into actionable points.

use crate::models::classified_line::ClassifiedLine;
use crate::models::comment_point::CommentPoint;

const FENCE: &str = "```";
const HEADING_MARK: char = '#';
const MIN_HEADING_LEVEL: usize = 2;
const MAX_HEADING_LEVEL: usize = 4;
const BULLET_MARKERS: [&str; 2] = ["- ", "* "];
const ORDERED_SUFFIX: &str = ". ";

fn heading_title(line: &str) -> Option<String> {
    let level = line.chars().take_while(|c| *c == HEADING_MARK).count();
    if !(MIN_HEADING_LEVEL..=MAX_HEADING_LEVEL).contains(&level) {
        return None;
    }
    let rest = line[level..].strip_prefix(' ')?;
    Some(rest.trim().to_string())
}

fn is_item(line: &str) -> bool {
    if BULLET_MARKERS.iter().any(|marker| line.starts_with(marker)) {
        return true;
    }
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    digits > 0 && line[digits..].starts_with(ORDERED_SUFFIX)
}

fn classify(markdown: &str) -> Vec<ClassifiedLine<'_>> {
    let mut in_fence = false;
    markdown
        .lines()
        .map(|text| {
            let fence_line = text.trim_start().starts_with(FENCE);
            let outside = !in_fence && !fence_line;
            if fence_line {
                in_fence = !in_fence;
            }
            ClassifiedLine {
                text,
                heading: if outside { heading_title(text) } else { None },
                item: outside && is_item(text),
            }
        })
        .collect()
}

fn push_point(points: &mut Vec<CommentPoint>, lines: &[&str], title: Option<String>) {
    let body = lines.join("\n").trim().to_string();
    if body.is_empty() {
        return;
    }
    points.push(CommentPoint {
        id: None,
        index: points.len(),
        title,
        body,
    });
}

fn cut(
    lines: &[ClassifiedLine<'_>],
    starts_point: impl Fn(&ClassifiedLine<'_>) -> bool,
) -> Vec<CommentPoint> {
    let mut points = Vec::new();
    let mut chunk: Vec<&str> = Vec::new();
    let mut title: Option<String> = None;
    for line in lines {
        if starts_point(line) {
            push_point(&mut points, &chunk, title.take());
            chunk.clear();
            title = line.heading.clone();
        }
        chunk.push(line.text);
    }
    push_point(&mut points, &chunk, title);
    points
}

/// Cuts on `##`-`####` headings, else on column-0 list items, else keeps one
/// point. Fenced blocks are never cut.
pub fn split_points(markdown: &str) -> Vec<CommentPoint> {
    let lines = classify(markdown);
    let cut_points = if lines.iter().any(|line| line.heading.is_some()) {
        cut(&lines, |line| line.heading.is_some())
    } else if lines.iter().any(|line| line.item) {
        cut(&lines, |line| line.item)
    } else {
        Vec::new()
    };
    if !cut_points.is_empty() {
        return cut_points;
    }
    vec![CommentPoint {
        id: None,
        index: 0,
        title: None,
        body: markdown.trim().to_string(),
    }]
}

#[cfg(test)]
#[path = "../tests/unit/points_tests.rs"]
mod tests;
