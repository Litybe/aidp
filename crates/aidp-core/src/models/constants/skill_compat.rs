//! Names used to read the `aidp` version a skill requires.

/// The skill's manifest, whose frontmatter declares the required version.
pub const SKILL_MANIFEST_FILE: &str = "SKILL.md";
/// Byte order mark an editor may put before the frontmatter.
pub const BYTE_ORDER_MARK: char = '\u{feff}';
/// Line opening and closing a Markdown frontmatter block.
pub const FRONTMATTER_DELIMITER: &str = "---";
/// Separator between the numeric parts of a version.
pub const VERSION_SEPARATOR: char = '.';
/// Start of a pre-release or build suffix, ignored when comparing versions.
pub const VERSION_SUFFIX_MARKERS: [char; 2] = ['-', '+'];
