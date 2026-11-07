//! Pure ticket workflow engine: flow model, validation, condition
//! evaluator and the `advance` state machine. No I/O of any kind.

pub mod error;
pub mod expr;
pub mod machine;
pub mod model;
pub mod models;
pub mod template;
pub mod validate;

pub use error::{FlowError, RejectReason};
pub use expr::{parse, Context, Expr};
pub use machine::{
    advance, effect_id, ActionOutcome, Effect, EffectKind, EffectName, Event, FlowEvent,
    HumanDecision, ManualRequest, PipelineRunStatus, StepReason, FLOW_LABEL, MANUAL_PREFIX,
    OUTPUT_REPORT,
};
pub use model::{
    FlowDef, FlowState, FlowStatus, Frame, FrameKind, OnError, Step, StepKind, StepState,
    StepStatus,
};
pub use validate::{validate, Catalog, Issue, IssueCode, Report};
