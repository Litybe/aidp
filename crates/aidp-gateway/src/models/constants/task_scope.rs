//! Git arguments and status markers used by the per-task write attribution.

pub const STATUS_ARG: &str = "status";
pub const PORCELAIN_FLAG: &str = "--porcelain";
pub const NUL_SEPARATED_FLAG: &str = "-z";
pub const UNTRACKED_ALL_FLAG: &str = "-uall";
pub const REV_PARSE_ARG: &str = "rev-parse";
pub const HEAD_REF: &str = "HEAD";
pub const STATUS_CODE_WIDTH: usize = 2;
pub const STATUS_PREFIX_WIDTH: usize = 3;
pub const RENAME_MARKER: char = 'R';
pub const COPY_MARKER: char = 'C';
pub const NUL_SEPARATOR: char = '\u{0}';
