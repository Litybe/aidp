/// Line numbers one hunk exposes on each side of the diff.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiffHunk {
    pub left_lines: Vec<u32>,
    pub right_lines: Vec<u32>,
}
