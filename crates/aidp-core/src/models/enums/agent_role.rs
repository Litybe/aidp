use super::effort_level::EffortLevel;
use super::model_tier::ModelTier;
use serde::{Deserialize, Serialize};

/// Sub-agent role, as a stage's `agent:` names it. `Custom` is any other name, whose registry
/// name lives in the stage's `agent_type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentRole {
    /// Reads code, docs and tickets, and synthesises; writes only its stage dir.
    Analyzer,
    /// Turns analysis and requirements into a plan; writes only its stage dir.
    Planner,
    /// Produces the actual artefact, and the role a stage without `agent:` runs as.
    Executor,
    /// Audits an artefact and emits structured findings.
    Reviewer,
    #[serde(other)]
    Custom,
}

impl AgentRole {
    /// Parses a stage's `agent:` value, mapping any unrecognised name to `Custom`.
    pub fn parse(raw: &str) -> Self {
        match raw {
            "analyzer" => Self::Analyzer,
            "planner" => Self::Planner,
            "executor" => Self::Executor,
            "reviewer" => Self::Reviewer,
            _ => Self::Custom,
        }
    }

    /// Wire value, identical to the serialized form.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Analyzer => "analyzer",
            Self::Planner => "planner",
            Self::Executor => "executor",
            Self::Reviewer => "reviewer",
            Self::Custom => "custom",
        }
    }

    /// Model a stage runs on when it pins none: planners get the strongest one.
    pub const fn default_model(&self) -> ModelTier {
        match self {
            Self::Planner => ModelTier::Opus,
            _ => ModelTier::Sonnet,
        }
    }

    /// Effort a stage runs with when it pins none and per-role efforts are emitted.
    pub const fn default_effort(&self) -> EffortLevel {
        match self {
            Self::Planner => EffortLevel::High,
            _ => EffortLevel::Medium,
        }
    }
}

impl std::fmt::Display for AgentRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
