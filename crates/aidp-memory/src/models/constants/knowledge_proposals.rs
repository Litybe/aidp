use crate::models::enums::Domain;

/// File a pipeline stage writes its knowledge proposals into.
pub const KNOWLEDGE_PROPOSALS_FILE: &str = "knowledge-proposals.json";
/// Most proposals accepted in one `knowledge-proposals.json`.
pub const MAX_KNOWLEDGE_PROPOSALS: usize = 5;
/// Longest proposed note body, in characters.
pub const MAX_NOTE_BODY_CHARS: usize = 8000;
/// Longest proposed note title, in characters.
pub const MAX_NOTE_TITLE_CHARS: usize = 120;
/// Longest evidence text of a proposal, in characters.
pub const MAX_EVIDENCE_CHARS: usize = 1000;
/// Cosine from which a proposal only adds evidence to the note it duplicates.
pub const MERGE_COSINE: f64 = 0.92;
/// Cosine from which a proposal goes to the inbox as a candidate of an existing note.
pub const INBOX_COSINE: f64 = 0.75;
/// Word-set Jaccard from which, without embeddings, a proposal duplicates a note.
pub const FALLBACK_JACCARD: f64 = 0.55;
/// BM25 hits compared with a proposal when no embedding model is installed.
pub const FALLBACK_CANDIDATES: usize = 5;
/// Importance of a proposal that gives none.
pub const DEFAULT_PROPOSAL_IMPORTANCE: f64 = 0.5;
/// Domains whose proposals always wait in the inbox for a human.
pub const INBOX_ONLY_DOMAINS: &[Domain] = &[Domain::Lore, Domain::GameDesign, Domain::Rooms];

/// Folder of the per-game notes under `<vault>/Knowledge/<repo>`.
pub const NOTES_SUBDIR: &str = "notes";
/// Folder of the previous versions of rewritten notes under `<vault>/Knowledge`; never indexed.
pub const HISTORY_DIR: &str = "_history";
/// Folder of the rejected proposals under the inbox.
pub const REJECTED_DIR: &str = "_rejected";
/// Folder of the inbox updates already applied to their target.
pub const APPLIED_DIR: &str = "_applied";
/// Extension of a note.
pub const NOTE_EXTENSION: &str = "md";
/// Suffix inserted before the extension of a name already taken.
pub const NAME_COLLISION_SEPARATOR: char = '-';
/// Separator replacing the folders in the name of a history file.
pub const HISTORY_NAME_SEPARATOR: &str = "__";

/// Heading of the evidence section of a note.
pub const EVIDENCE_HEADING: &str = "## Evidence";
/// Prefix of a section heading.
pub const SECTION_PREFIX: &str = "## ";
/// Prefix of the title heading.
pub const TITLE_HEADING_PREFIX: &str = "# ";
/// `type` of a knowledge note.
pub const NOTE_TYPE: &str = "note";
/// `source` of a note ingested by hand.
pub const MANUAL_SOURCE: &str = "manual";
/// Prefix of the `source` of a note proposed by a run.
pub const RUN_SOURCE_PREFIX: &str = "run:";
/// Format of the `updated` date.
pub const DATE_FORMAT: &str = "%Y-%m-%d";
/// Opening of a wikilink.
pub const WIKILINK_OPEN: &str = "[[";
/// Closing of a wikilink.
pub const WIKILINK_CLOSE: &str = "]]";
/// Prefix of the folders of a vault path.
pub const FOLDER_SEPARATOR: char = '/';
/// Bullet of an evidence line.
pub const EVIDENCE_BULLET: &str = "- ";

/// Frontmatter key: lifecycle state.
pub const KEY_STATUS: &str = "status";
/// Frontmatter key: date of the last change.
pub const KEY_UPDATED: &str = "updated";
/// Frontmatter key: where the note comes from.
pub const KEY_SOURCE: &str = "source";
/// Frontmatter key: the note that replaced this one.
pub const KEY_SUPERSEDED_BY: &str = "superseded_by";
/// Frontmatter key: the existing note an inbox note resembles.
pub const KEY_CANDIDATE_OF: &str = "candidate_of";
/// Frontmatter key: what approving an inbox note does.
pub const KEY_ACTION: &str = "action";
/// Frontmatter key: where an inbox note will live.
pub const KEY_SCOPE: &str = "scope";
/// Frontmatter key: the note an inbox update or supersede applies to.
pub const KEY_TARGET: &str = "target";
/// Frontmatter key: the game folder of a project-scoped inbox note.
pub const KEY_REPO: &str = "repo";
/// Frontmatter keys only an inbox note carries.
pub const INBOX_KEYS: [&str; 5] = [
    KEY_ACTION,
    KEY_SCOPE,
    KEY_TARGET,
    KEY_CANDIDATE_OF,
    KEY_REPO,
];
