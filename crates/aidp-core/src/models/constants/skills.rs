//! Keys and separators used to read skills-loading signals from reply envelopes.

/// Envelope key holding the reply's signals object.
pub const SIGNALS_KEY: &str = "signals";
/// Signal key listing the declared skills a reply did not load.
pub const SKILLS_NOT_LOADED_KEY: &str = "skills_not_loaded";
/// Separator between a plan stage name and its task id in a result key.
pub const PLAN_TASK_SEPARATOR: char = '#';
/// Skill that only has a purpose when the run carries a knowledge graph.
pub const GRAPHIFY_CONTEXT_SKILL: &str = "graphify-context";
/// Prefix of the compile warning naming a stage that lost an unavailable skill.
pub const SKILL_DROPPED_WARNING_PREFIX: &str = "skill dropped, no knowledge graph for this run";
/// Separator between the warning prefix and the skill name.
pub const SKILL_DROPPED_WARNING_SEPARATOR: &str = ": ";
/// Text between the skill name and the stage name in the drop warning.
pub const SKILL_DROPPED_WARNING_STAGE_TEXT: &str = " removed from stage ";
