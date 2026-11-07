use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("schema violation: {0}")]
    SchemaViolation(String),
    #[error("scope violation: {0}")]
    ScopeViolation(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("yaml: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("not found: {0}")]
    NotFound(String),
    #[error("conflict: {0}")]
    Conflict(String),
    #[error("unauthorized: {0}")]
    Unauthorized(String),
    #[error("config: {0}")]
    Config(String),
    #[error("upstream: {0}")]
    Upstream(String),
    #[error("{0}")]
    Other(String),
}
