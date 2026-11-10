use crate::models::enums::MemoryKind;

/// Array of proposals at the root of a proposals file.
pub const FIELD_PROPOSALS: &str = "proposals";
/// Documented field holding the kind of a proposal.
pub const FIELD_KIND: &str = "kind";
/// Documented field holding the title of a proposal.
pub const FIELD_TITLE: &str = "title";
/// Documented field holding the text of a proposal.
pub const FIELD_TEXT: &str = "text";
/// Undocumented field agents use for the kind.
pub const FIELD_LEGACY_TYPE: &str = "type";
/// Any of these marks a proposal written in an undocumented shape.
pub const LEGACY_MARKERS: [&str; 9] = [
    FIELD_LEGACY_TYPE,
    "name",
    "description",
    "body",
    "lesson",
    "fact",
    "why",
    "how_to_apply",
    "evidence",
];
/// Fields that can hold the title, best first.
pub const TITLE_ALIASES: [&str; 3] = [FIELD_TITLE, "description", "name"];
/// Fields that can hold the text, best first.
pub const TEXT_ALIASES: [&str; 4] = [FIELD_TEXT, "body", "lesson", "fact"];
/// Extra fields appended to the text, with the label of their line.
pub const DETAIL_FIELDS: [(&str, &str); 3] = [
    ("why", "Why"),
    ("how_to_apply", "How"),
    ("evidence", "Evidence"),
];
/// Undocumented types with a closer kind than the default.
pub const LEGACY_TYPE_KINDS: [(&str, MemoryKind); 3] = [
    ("user", MemoryKind::Preference),
    ("project", MemoryKind::Fact),
    ("reference", MemoryKind::Fact),
];
/// Kind of a proposal in an undocumented shape with no recognised type.
pub const LEGACY_DEFAULT_KIND: MemoryKind = MemoryKind::Lesson;
/// End of a title or text shortened to its limit.
pub const TRUNCATION_MARK: &str = "...";
/// Path segment naming the issue of a pipeline run.
pub const ISSUE_SEGMENT_PATTERN: &str = r"^GH-\d+$";
/// Path segment naming a pipeline run.
pub const RUN_SEGMENT_PATTERN: &str = r"^\d{8}-\d{6}-[0-9a-f]{4}$";
