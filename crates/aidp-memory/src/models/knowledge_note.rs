use crate::models::note_frontmatter::NoteFrontmatter;

/// A knowledge file read for indexing: its frontmatter and its title.
#[derive(Debug, Clone, PartialEq)]
pub struct KnowledgeNote {
    pub front: NoteFrontmatter,
    pub title: String,
}
