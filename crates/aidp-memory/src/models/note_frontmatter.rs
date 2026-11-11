use serde::Deserialize;

/// The frontmatter fields of a knowledge note the index keeps.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct NoteFrontmatter {
    #[serde(rename = "type")]
    pub note_type: Option<String>,
    pub title: Option<String>,
    pub domain: Option<String>,
    pub tags: Vec<String>,
    pub agents: Vec<String>,
    pub status: Option<String>,
    pub importance: Option<f64>,
    pub updated: Option<String>,
}
