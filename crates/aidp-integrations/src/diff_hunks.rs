//! Unified-diff hunk parsing and the check that a comment anchor lies inside it.

use crate::models::constants::review_post::{
    DIFF_ADDED, DIFF_NO_NEWLINE, DIFF_REMOVED, HUNK_END, HUNK_PREFIX, HUNK_RANGE_SEPARATOR,
    HUNK_RIGHT_SEPARATOR,
};
use crate::models::diff_hunk::DiffHunk;
use crate::models::enums::diff_side::DiffSide;

fn range_start(range: &str) -> Option<u32> {
    range.split(HUNK_RANGE_SEPARATOR).next()?.parse().ok()
}

fn hunk_starts(line: &str) -> Option<(u32, u32)> {
    let ranges = line.strip_prefix(HUNK_PREFIX)?.split(HUNK_END).next()?;
    let (left, right) = ranges.split_once(HUNK_RIGHT_SEPARATOR)?;
    Some((range_start(left)?, range_start(right)?))
}

/// Parses a file patch into hunks: deleted and context lines in old numbering, added and context lines in new.
pub fn parse_hunks(patch: &str) -> Vec<DiffHunk> {
    let mut hunks: Vec<DiffHunk> = Vec::new();
    let (mut left, mut right) = (0, 0);
    for line in patch.lines() {
        if let Some((left_start, right_start)) = hunk_starts(line) {
            (left, right) = (left_start, right_start);
            hunks.push(DiffHunk::default());
            continue;
        }
        let Some(current) = hunks.last_mut() else {
            continue;
        };
        match line.chars().next() {
            Some(DIFF_ADDED) => {
                current.right_lines.push(right);
                right += 1;
            }
            Some(DIFF_REMOVED) => {
                current.left_lines.push(left);
                left += 1;
            }
            Some(DIFF_NO_NEWLINE) => {}
            _ => {
                current.left_lines.push(left);
                current.right_lines.push(right);
                left += 1;
                right += 1;
            }
        }
    }
    hunks
}

fn side_lines(hunk: &DiffHunk, side: DiffSide) -> &[u32] {
    match side {
        DiffSide::Left => &hunk.left_lines,
        DiffSide::Right => &hunk.right_lines,
    }
}

/// True when `line`, and the `start_line` of a range, both sit in one hunk on `side`.
pub fn anchor_in_hunks(
    hunks: &[DiffHunk],
    line: u32,
    side: DiffSide,
    start_line: Option<u32>,
) -> bool {
    let start = start_line.unwrap_or(line);
    start <= line
        && hunks.iter().any(|hunk| {
            let lines = side_lines(hunk, side);
            lines.contains(&start) && lines.contains(&line)
        })
}

#[cfg(test)]
#[path = "../tests/unit/diff_hunks_tests.rs"]
mod tests;
