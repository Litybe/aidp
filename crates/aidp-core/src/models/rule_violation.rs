//! One added line that breaks a skill rule.

use crate::models::enums::skill_rule::SkillRule;
use serde::Serialize;

/// A rule broken by an added line, located in the new file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RuleViolation {
    pub rule: SkillRule,
    pub file: String,
    pub line: u32,
    pub excerpt: String,
}
