use crate::models::enums::Source;

/// Largest Markdown or plain-text chunk, in characters.
pub const MARKDOWN_MAX_CHARS: usize = 1200;
/// Characters repeated at the start of the next Markdown chunk of one section.
pub const MARKDOWN_OVERLAP_CHARS: usize = 150;
/// Largest C# chunk, in characters.
pub const CODE_MAX_CHARS: usize = 1800;
/// Files above this size are not indexed.
pub const MAX_FILE_BYTES: u64 = 1_000_000;
/// Characters per token in every budget computation.
pub const CHARS_PER_TOKEN: usize = 4;
/// Longest memory title, in characters.
pub const TITLE_MAX_CHARS: usize = 120;
/// Longest memory text, in characters.
pub const TEXT_MAX_CHARS: usize = 600;
/// Most proposals accepted in one `memory-proposals.json`.
pub const MAX_PROPOSALS_PER_FILE: usize = 5;
/// Default number of search results.
pub const DEFAULT_K: usize = 20;
/// Default `rag search` budget, in tokens.
pub const DEFAULT_SEARCH_BUDGET: usize = 1500;
/// Default `memory context` budget, in tokens.
pub const DEFAULT_CONTEXT_BUDGET: usize = 2000;
/// Share of the context budget the memories may use.
pub const MEMORY_BUDGET_SHARE: f64 = 0.4;
/// MMR trade-off: 1.0 is pure relevance, 0.0 pure diversity.
pub const MMR_LAMBDA: f64 = 0.7;
/// Candidates kept before the MMR pass, as a multiple of `k`.
pub const CANDIDATE_FACTOR: usize = 5;
/// Fewest candidates fetched from the index.
pub const MIN_CANDIDATES: usize = 60;
/// Candidates fetched when a path filter is applied after the query.
pub const FILTERED_CANDIDATES: usize = 400;
/// Score multiplier of a chunk reached through the code graph.
pub const GRAPH_DAMPING: f64 = 0.5;
/// Hit chunks used as graph seeds.
pub const GRAPH_SEED_HITS: usize = 8;
/// Neighbours followed per seed symbol.
pub const GRAPH_NEIGHBOURS_PER_SEED: usize = 5;
/// Most chunks added by the graph expansion.
pub const GRAPH_MAX_ADDED: usize = 30;
/// Jaccard similarity from which a proposal duplicates a memory.
pub const DEDUP_SIMILARITY: f64 = 0.55;
/// Memories fetched as dedup candidates.
pub const DEDUP_CANDIDATES: usize = 8;
/// Evidence needed for the automatic activation.
pub const AUTO_ACTIVATE_EVIDENCE: u32 = 2;
/// Distinct issues needed for the automatic activation.
pub const AUTO_ACTIVATE_ISSUES: usize = 2;
/// Confidence needed for the automatic activation.
pub const AUTO_ACTIVATE_CONFIDENCE: f64 = 0.8;
/// Confidence from which a memory written by an automatic source is active at once.
pub const AUTO_SOURCE_ACTIVATE_CONFIDENCE: f64 = 0.7;
/// Days without use after which an active memory goes stale.
pub const STALE_AFTER_DAYS: i64 = 60;
/// Confidence given to a memory added without one.
pub const DEFAULT_CONFIDENCE: f64 = 0.7;
/// Most active memories considered for one context block.
pub const MAX_CONTEXT_MEMORIES: usize = 30;
/// Most query terms sent to the index.
pub const MAX_QUERY_TERMS: usize = 24;
/// Most identifier tokens kept per chunk.
pub const MAX_IDENT_TOKENS: usize = 400;
/// Shortest term that gets a prefix match.
pub const PREFIX_MIN_CHARS: usize = 3;
/// Terms longer than this lose a trailing plural mark.
pub const PLURAL_MIN_CHARS: usize = 5;
/// Directory depth explored by `memory ingest` under a folder.
pub const INGEST_MAX_DEPTH: usize = 6;
/// Busy timeout of the database, in milliseconds.
pub const BUSY_TIMEOUT_MS: u64 = 5000;
/// Score multiplier per source: memories and the canon first, run artefacts last.
pub const SOURCE_WEIGHTS: [(Source, f64); 7] = [
    (Source::Memory, 1.5),
    (Source::Bible, 1.2),
    (Source::Doc, 1.0),
    (Source::Code, 1.0),
    (Source::Knowledge, 0.9),
    (Source::Ticket, 0.7),
    (Source::Run, 0.4),
];
/// Path segment of the session journals.
pub const JOURNAL_SEGMENT: &str = "/_journal/";
/// Extra multiplier of a session journal.
pub const JOURNAL_WEIGHT: f64 = 0.5;
