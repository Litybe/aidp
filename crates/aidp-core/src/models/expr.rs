//! Evaluation context shared by the expression engine.

use crate::pipeline::{Pipeline, Stage};
use crate::run::RunState;

/// Evaluation context: run state and pipeline, plus an optional loop body.
pub struct EvalCtx<'a> {
    pub state: &'a RunState,
    pub pipe: &'a Pipeline,
    pub loop_body: Option<&'a [Stage]>,
}

impl<'a> EvalCtx<'a> {
    pub fn new(state: &'a RunState, pipe: &'a Pipeline) -> Self {
        Self {
            state,
            pipe,
            loop_body: None,
        }
    }

    pub fn with_loop_body(mut self, body: Option<&'a [Stage]>) -> Self {
        self.loop_body = body;
        self
    }
}
