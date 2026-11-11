use serde::Deserialize;

/// The frontmatter fields of a note waiting in the inbox.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct InboxFrontmatter {
    pub domain: Option<String>,
    pub action: Option<String>,
    pub scope: Option<String>,
    pub repo: Option<String>,
    pub target: Option<String>,
    pub candidate_of: Option<String>,
    pub source: Option<String>,
    pub updated: Option<String>,
}
