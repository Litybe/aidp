//! Declarative catalogue of mechanical rules checked on added diff lines.

use crate::models::constants::skill_rules::{
    ALL_SKILL_RULES, NO_LINE_COMMENT_ID, NO_LINE_COMMENT_MESSAGE, NO_TICKET_REFERENCE_ID,
    NO_TICKET_REFERENCE_MESSAGE, RUST_DEV_SKILL, RUST_EXTENSION,
};
use serde::Serialize;

/// One mechanical rule; adding a variant adds a rule with no engine change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum SkillRule {
    NoLineComment,
    NoTicketReference,
}

impl SkillRule {
    /// Every rule of the catalogue.
    pub fn all() -> &'static [SkillRule] {
        ALL_SKILL_RULES
    }

    /// Stable machine id of the rule.
    pub fn id(&self) -> &'static str {
        match self {
            SkillRule::NoLineComment => NO_LINE_COMMENT_ID,
            SkillRule::NoTicketReference => NO_TICKET_REFERENCE_ID,
        }
    }

    /// Name of the skill that owns the rule.
    pub fn skill(&self) -> &'static str {
        match self {
            SkillRule::NoLineComment | SkillRule::NoTicketReference => RUST_DEV_SKILL,
        }
    }

    /// Whether the rule checks the file at `path`.
    pub fn applies_to(&self, path: &str) -> bool {
        match self {
            SkillRule::NoLineComment | SkillRule::NoTicketReference => {
                path.ends_with(RUST_EXTENSION)
            }
        }
    }

    /// Human text describing the violation.
    pub fn message(&self) -> &'static str {
        match self {
            SkillRule::NoLineComment => NO_LINE_COMMENT_MESSAGE,
            SkillRule::NoTicketReference => NO_TICKET_REFERENCE_MESSAGE,
        }
    }
}
