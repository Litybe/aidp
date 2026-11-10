pub const AWAITING_PLAN_NOTE: &str = "the tail of this workflow is compiled from the plan — run `aidp workflow emit` again once the planner has written it, then resume with resumeFromRunId";

pub const AWAITING_COMPOSITE_NOTE: &str = "this composite starts a new Workflow pass — `aidp run record-resume --crossed-boundary <composite>`, then run `aidp workflow emit` again and resume with resumeFromRunId";

pub const PHASE_AWAITING_PLAN: &str = "awaiting_plan";

pub const PHASE_PLAN_COMPILED: &str = "plan_compiled";

pub const PHASE_AWAITING_COMPOSITE: &str = "awaiting_composite";

pub mod embedded_command;
pub mod flow_git;
pub mod flow_pr;
pub mod flow_unity;
pub mod memory_hook;
