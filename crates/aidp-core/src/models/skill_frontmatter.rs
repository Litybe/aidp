use serde::Deserialize;

/// The frontmatter keys of a skill's `SKILL.md` that `aidp` reads; every other key is ignored.
#[derive(Debug, Default, Deserialize)]
pub struct SkillFrontmatter {
    /// Oldest `aidp` version the skill works with.
    pub min_aidp_version: Option<String>,
}
