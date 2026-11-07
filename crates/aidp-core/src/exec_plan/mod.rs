//! Typed execution plan: what one Workflow pass runs, compiled from a pipeline and a run state.

mod builder;
pub(crate) mod refs;

pub use crate::models::enums::contract_flag::ContractFlag;
pub use crate::models::enums::fanout_items::FanoutItems;
pub use crate::models::enums::pass_end::PassEnd;
pub use crate::models::enums::reply_schema_kind::ReplySchemaKind;
pub use crate::models::enums::skip_reason::SkipReason;
pub use crate::models::enums::step::Step;
pub use crate::models::exec_plan::ExecPlan;
pub use crate::models::gate_spec::GateSpec;
pub use crate::models::stage_spec::StageSpec;
pub use builder::compile_exec_plan;

#[cfg(test)]
#[path = "../../tests/unit/exec_plan_schema_tests.rs"]
mod schema_tests;
#[cfg(test)]
#[path = "../../tests/unit/support/mod.rs"]
pub(crate) mod test_support;
#[cfg(test)]
#[path = "../../tests/unit/exec_plan_tests.rs"]
mod tests;
