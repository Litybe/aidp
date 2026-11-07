use crate::models::enums::Source;
use std::path::PathBuf;

/// A directory outside the project (vault runs, tickets, knowledge) feeding one source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexRoot {
    pub source: Source,
    pub dir: PathBuf,
    /// Prefix of the stored paths, such as `Pipelines/pixelhood`.
    pub prefix: String,
}
