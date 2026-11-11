/// Model used when the caller names none.
pub const DEFAULT_MODEL_NAME: &str = "potion-multilingual-128M";
/// Folder of the embedding models under `<gateway>/memory`.
pub const MODELS_DIR: &str = "models";
/// Files a Model2Vec model folder must hold.
pub const MODEL_FILES: [&str; 3] = ["model.safetensors", "tokenizer.json", "config.json"];
/// Reciprocal rank fusion constant.
pub const RRF_K: f64 = 60.0;
/// Chunks embedded per call to the model.
pub const EMBED_BATCH: usize = 64;
/// Longest text sent to the model per chunk, in characters.
pub const EMBED_MAX_CHARS: usize = 2000;
/// Candidates the vector side contributes, as a multiple of `k`.
pub const VECTOR_CANDIDATE_FACTOR: usize = 5;
/// Bytes of one stored vector component.
pub const BYTES_PER_COMPONENT: usize = 4;
/// Vector similarity below which a chunk is not a candidate.
pub const MIN_COSINE: f64 = 0.05;
/// Via value of a hit found by vectors only.
pub const VIA_VECTOR: &str = "vector";
/// Via value of a hit found by both BM25 and vectors.
pub const VIA_HYBRID: &str = "hybrid";
