use crate::models::enums::MemoryKind;

/// Line that opens and closes the frontmatter of a note.
pub const FRONTMATTER_FENCE: &str = "---";
/// Start of the heading line that holds the title.
pub const TITLE_PREFIX: &str = "# ";
/// Folder of the notes with no route.
pub const NO_ROUTE_DIR: &str = "general";
/// Folder of the archived and superseded notes.
pub const ARCHIVE_DIR: &str = "_archive";
/// Longest title excerpt in a note file name.
pub const SLUG_MAX_CHARS: usize = 48;
/// Separator of the words in a note file name.
pub const SLUG_SEPARATOR: char = '-';
/// Separator of the type and the id in a note reference.
pub const NOTE_REF_SEPARATOR: char = ':';
/// Kind of a hand-written note that names none.
pub const NOTE_DEFAULT_KIND: MemoryKind = MemoryKind::Fact;
/// Id at the start of a note file name.
pub const NOTE_FILE_ID_PATTERN: &str = r"^M-(\d+)";
