//! Definitions shared across the core.

pub mod assumption;
pub(crate) mod build_ctx;
pub(crate) mod build_out;
#[cfg(test)]
pub(crate) mod case_task;
pub mod constants;
pub mod enums;
pub mod escalation;
pub mod exec_plan;
pub mod expr;
pub mod finding_location;
pub mod gate_spec;
pub mod git;
pub mod knowledge_settings;
pub(crate) mod loop_request;
pub(crate) mod loop_signal;
pub mod pipeline;
pub(crate) mod plan_request;
pub mod port;
pub mod process;
pub mod prompt_form;
pub mod rule_violation;
pub(crate) mod run_started_at;
pub mod run_state;
pub(crate) mod run_stub;
pub mod scope;
pub mod skill_frontmatter;
pub mod stack_inputs;
pub mod stage_context_request;
pub mod stage_spec;
pub mod typed_exit;
pub mod vault;
pub mod workflow_emit;

pub use typed_exit::TypedExit;
