/// Ordered schema migrations; `PRAGMA user_version` holds how many ran.
pub const MIGRATIONS: &[&str] = &[
    include_str!("../../../migrations/0001_initial.sql"),
    include_str!("../../../migrations/0002_note_hash.sql"),
];
/// Project key of the memories that apply to every project.
pub const GLOBAL_PROJECT: &str = "_global";
/// Tokenizer of every full-text table.
pub const FTS_TOKENIZER: &str = "unicode61 remove_diacritics 2";
/// BM25 column weights of `chunks_fts`: title, text, identifiers, path.
pub const CHUNK_WEIGHTS: &str = "5.0, 1.0, 2.0, 1.5";
/// BM25 column weights of `memories_fts`: title, text.
pub const MEMORY_WEIGHTS: &str = "3.0, 1.0";
/// Identifier prefix of a memory.
pub const MEMORY_ID_PREFIX: &str = "M-";
/// Chunk id prefix of an indexed memory.
pub const MEMORY_CHUNK_PREFIX: &str = "memory:";
/// Path prefix of an indexed memory.
pub const MEMORY_PATH_PREFIX: &str = "memory/";
