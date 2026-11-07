//! Durations, status keys, wire values and output keys of the PR wait probes.

pub const REVIEWS_QUIET_MS: i64 = 10 * 60 * 1000;

pub const KEY_NAME: &str = "name";
pub const KEY_REVIEWERS: &str = "reviewers";
pub const KEY_LOGIN: &str = "login";
pub const KEY_DECISION: &str = "decision";
pub const KEY_UNRESOLVED: &str = "unresolved";
pub const KEY_OK: &str = "ok";

pub const OUTPUT_CONCLUSION: &str = "conclusion";
pub const OUTPUT_THREADS: &str = "threads";
pub const OUTPUT_NEW_THREADS: &str = "new_threads";
pub const OUTPUT_APPROVED: &str = "approved";

pub const KEY_THREADS: &str = "threads";
pub const KEY_REVIEW: &str = "review";
pub const KEY_ACTIONABLE: &str = "actionable";
pub const KEY_ACTIONABLE_IDS: &str = "actionableIds";
pub const OUTPUT_CLEAN: &str = "clean";
