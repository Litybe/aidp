//! Names of the embedding model service of the daemon.

/// Event published on every state change of the model; its payload is the status body.
pub const EVENT_MEMORY_MODEL: &str = "memory.model";
/// Thread that loads the model.
pub const MODEL_LOAD_THREAD: &str = "memory-model-load";
/// Error shown when the loading thread panicked.
pub const MODEL_LOAD_PANIC: &str = "the model load panicked";
/// Error shown when the loading thread could not be started.
pub const MODEL_THREAD_FAILED: &str = "the model load thread did not start";
/// Wire code of a start request for a model that is not installed.
pub const CODE_MODEL_MISSING: &str = "model_missing";
/// Wire code of a request that needs the running model.
pub const CODE_MODEL_STOPPED: &str = "model_stopped";
/// Wire code of a body or batch that is too large.
pub const CODE_BATCH_TOO_LARGE: &str = "batch_too_large";
/// Trigger of the re-index job that embeds the chunks lacking a vector.
pub const TRIGGER_EMBED_PENDING: &str = "embed-pending";
/// Path segment of the model routes under `/api/memory`.
pub const SEGMENT_MODEL: &str = "model";
/// Path segment that starts the model.
pub const SEGMENT_START: &str = "start";
/// Path segment that stops the model.
pub const SEGMENT_STOP: &str = "stop";
/// Path segment that embeds the chunks lacking a vector.
pub const SEGMENT_EMBED_PENDING: &str = "embed-pending";
/// Path segment of `POST /api/embed`.
pub const SEGMENT_EMBED: &str = "embed";
