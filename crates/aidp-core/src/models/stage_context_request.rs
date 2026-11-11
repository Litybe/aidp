use crate::models::enums::KnowledgeRoute;
use std::path::PathBuf;

/// What a [`crate::context_provider::ContextProvider`] builds the knowledge block of one stage from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StageContextRequest {
    pub project_root: PathBuf,
    pub stage_id: String,
    pub stage_description: String,
    /// The agent the stage runs as.
    pub agent_type: String,
    pub route: Option<KnowledgeRoute>,
    /// Issue or ticket the run works on, as the run inputs name it.
    pub issue: Option<String>,
    /// Text of the request, when the run inputs carry it.
    pub issue_text: Option<String>,
    pub budget_tokens: usize,
}
