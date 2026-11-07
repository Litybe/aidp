use std::path::PathBuf;

/// Skill-owned pipeline and flow definitions, without process-global configuration.
pub struct PipelineCatalog {
    pub(crate) root: PathBuf,
}
