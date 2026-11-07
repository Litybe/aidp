use std::path::PathBuf;

/// Why a native stage's `reply.md` was rejected.
#[derive(Debug, thiserror::Error)]
pub enum NativeReplyError {
    #[error("reply.md is missing at {}", .0.display())]
    Missing(PathBuf),
    #[error("reply.md could not be read: {0}")]
    Unreadable(String),
    #[error("reply.md holds no JSON envelope (whole file or a json fenced block): {0}")]
    InvalidJson(String),
    #[error("reply.md violates the reply schema: {0}")]
    SchemaViolation(String),
    #[error("reviewer reply needs signals.has_blocking_findings (boolean) and signals.unevaluated_checks (integer >= 0)")]
    ReviewerSignals,
    #[error("declared output `{0}` is absent from outputs")]
    MissingOutput(String),
    #[error("output `{name}` points to `{path}`, which does not exist")]
    OutputNotOnDisk { name: String, path: String },
}
