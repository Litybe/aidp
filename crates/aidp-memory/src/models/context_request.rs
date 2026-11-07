use crate::models::constants::limits::DEFAULT_CONTEXT_BUDGET;
use crate::models::enums::Route;

/// What `MemoryManager::context` builds a block for.
#[derive(Debug, Clone)]
pub struct ContextRequest {
    pub project: String,
    pub issue: Option<String>,
    /// Text of the issue, the query of the excerpt search.
    pub issue_text: Option<String>,
    pub route: Option<Route>,
    pub stage: Option<String>,
    pub budget_tokens: usize,
}

impl ContextRequest {
    /// A request with the default budget and no issue.
    pub fn new(project: impl Into<String>) -> Self {
        Self {
            project: project.into(),
            issue: None,
            issue_text: None,
            route: None,
            stage: None,
            budget_tokens: DEFAULT_CONTEXT_BUDGET,
        }
    }
}
