//! Literals of the skill rules diff checker.

use crate::models::enums::skill_rule::SkillRule;

/// Every rule of the catalogue.
pub const ALL_SKILL_RULES: &[SkillRule] = &[SkillRule::NoLineComment, SkillRule::NoTicketReference];
/// Iteration number stamped on generated findings.
pub const GENERATED_ITERATION: u32 = 1;
/// Schema version stamped on generated findings.
pub const GENERATED_SCHEMA_VERSION: i64 = 1;
/// First character of a context line in a unified diff.
pub const CONTEXT_PREFIX: char = ' ';

/// Skill that owns the Rust conventions rules.
pub const RUST_DEV_SKILL: &str = "rust-dev-aidp";
/// Stable id of the line comment rule.
pub const NO_LINE_COMMENT_ID: &str = "no-line-comment";
/// Stable id of the ticket reference rule.
pub const NO_TICKET_REFERENCE_ID: &str = "no-ticket-reference";
/// Extension of the files the Rust rules apply to.
pub const RUST_EXTENSION: &str = ".rs";
/// Prefix of a new-file header line in a unified diff.
pub const NEW_FILE_PREFIX: &str = "+++ b/";
/// Prefix of a hunk header line in a unified diff.
pub const HUNK_PREFIX: &str = "@@";
/// Prefix of an added line in a unified diff.
pub const ADDED_PREFIX: &str = "+";
/// Prefix of a removed line in a unified diff.
pub const REMOVED_PREFIX: &str = "-";
/// Prefix of a file header line, which is not an added line.
pub const FILE_HEADER_PREFIX: &str = "+++";
/// Marker that starts a line comment.
pub const LINE_COMMENT_MARKER: &str = "//";
/// Characters that turn a line comment marker into a doc comment.
pub const DOC_COMMENT_CHARS: [char; 2] = ['/', '!'];
/// Regex matching a ticket key.
pub const TICKET_PATTERN: &str = r"\bLOC-[0-9]+\b";
/// Prefix of the finding ids produced by the checker.
pub const FINDING_ID_PREFIX: &str = "F-SR-";
/// Finding category used by the checker.
pub const FINDING_CATEGORY: &str = "conventions";
/// Prefix of the check ids produced by the checker.
pub const CHECK_ID_PREFIX: &str = "C-SR-";
/// Message of the line comment rule.
pub const NO_LINE_COMMENT_MESSAGE: &str = "line comment in new Rust code; use /// or //! docs";
/// Message of the ticket reference rule.
pub const NO_TICKET_REFERENCE_MESSAGE: &str = "ticket reference in new Rust code";
/// Prefix of the start token of the new-file range in a hunk header.
pub const HUNK_NEW_RANGE_PREFIX: char = '+';
/// Separator between the start and the length of a hunk range.
pub const HUNK_RANGE_SEPARATOR: char = ',';
/// Text between the rule id and the skill name in a finding title.
pub const TITLE_SKILL_SEPARATOR: &str = " (skill ";
/// Text closing a finding title.
pub const TITLE_SKILL_END: &str = ")";
/// Path git prints for a deleted file.
pub const DEV_NULL_PATH: &str = "/dev/null";
