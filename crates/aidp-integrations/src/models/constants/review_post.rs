//! Marker, diff and message constants for posting review findings.

pub const MARKER_OPEN: &str = "<!-- aidp:finding";
pub const MARKER_CLOSE: &str = "-->";
pub const MARKER_ID_KEY: &str = "id";
pub const MARKER_RUN_KEY: &str = "run";
pub const MARKER_PASS_KEY: &str = "pass";
pub const MARKER_ASSIGN: char = '=';

pub const HUNK_PREFIX: &str = "@@ -";
pub const HUNK_END: &str = " @@";
pub const HUNK_RIGHT_SEPARATOR: &str = " +";
pub const HUNK_RANGE_SEPARATOR: char = ',';
pub const DIFF_ADDED: char = '+';
pub const DIFF_REMOVED: char = '-';
pub const DIFF_NO_NEWLINE: char = '\\';

pub const GH_API: &str = "api";
pub const HEAD_SHA_POINTER: &str = "/head/sha";
pub const FILES_PER_PAGE: u32 = 100;
pub const FIRST_PAGE: u32 = 1;
pub const MAX_FILE_PAGES: u32 = 30;

pub const THREADS_KEY: &str = "threads";
pub const THREAD_ID_KEY: &str = "thread_id";
pub const THREAD_PATH_KEY: &str = "path";
pub const THREAD_LINE_KEY: &str = "line";
pub const BODY_KEY: &str = "body";

pub const COMMENT_HEADING_OPEN: &str = "**[";
pub const COMMENT_HEADING_CLOSE: &str = "]**";
pub const DEFAULT_SEVERITY: &str = "info";
pub const REVIEW_SUMMARY: &str = "aidp review";
pub const REVIEW_OUT_OF_DIFF_HEADING: &str = "Findings outside the diff:";
pub const NO_LOCATION: &str = "no location";

pub const NO_HEAD_SHA: &str = "head commit of the pull request";
pub const INVALID_FINDINGS: &str = "findings file is not valid JSON";
pub const BAD_REVIEWS_REPLY: &str = "unexpected reviews reply";
