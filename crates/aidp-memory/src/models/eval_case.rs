use serde::{Deserialize, Serialize};

/// One retrieval question and the paths that answer it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EvalCase {
    #[serde(default)]
    pub query: String,
    /// Issue whose mirrored text is added to the query, as `memory context` does.
    #[serde(default)]
    pub issue: Option<String>,
    /// Path prefixes; the case passes when one of the first `k` hits starts with any of them.
    pub expect: Vec<String>,
    /// Sources to search, comma separated; every source when absent.
    #[serde(default)]
    pub sources: Option<String>,
    /// Route of the issue, filled from its ticket before the run.
    #[serde(skip)]
    pub issue_route: Option<crate::models::enums::Route>,
}
