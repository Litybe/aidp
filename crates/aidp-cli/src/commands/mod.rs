use aidp_core::{ExitStatus, Format};

pub mod agents;
pub mod cost;
pub mod doctor;
pub mod findings;
pub mod flow;
pub mod gate;
pub mod gateway;
pub mod gh;
pub mod git;
pub mod knowledge;
pub mod memory;
pub mod memory_hook;
pub mod memory_knowledge;
pub mod memory_model;
pub mod mock;
pub mod pipeline;
pub mod progress;
pub mod project;
pub mod run;
pub mod run_loader;
pub mod scope;
pub mod setup;
pub mod skill_rules;
pub mod skills;
pub mod stage_knowledge;
pub mod status;
pub mod ticket;
pub mod ticket_github;
pub mod topo;
pub mod validate;
pub mod vault;
pub mod workflow;

/// What a command hands back to `main` for printing.
pub enum Payload {
    Empty,
    Text(String),
    Json {
        value: serde_json::Value,
        format: Option<Format>,
    },
}

/// The result of a command: what to print, and how the process should end.
/// A command never prints; `failure` is reported by `main` after the payload.
pub struct CommandOutput {
    pub payload: Payload,
    pub diagnostics: Vec<String>,
    pub exit: Option<ExitStatus>,
    pub child_exit: Option<i32>,
    pub failure: Option<anyhow::Error>,
}

impl CommandOutput {
    pub fn empty() -> Self {
        Self {
            payload: Payload::Empty,
            diagnostics: Vec::new(),
            exit: None,
            child_exit: None,
            failure: None,
        }
    }

    pub fn text(text: impl Into<String>) -> Self {
        Self {
            payload: Payload::Text(text.into()),
            ..Self::empty()
        }
    }

    pub fn json(format: Option<Format>, value: serde_json::Value) -> Self {
        Self {
            payload: Payload::Json { value, format },
            ..Self::empty()
        }
    }

    #[cfg(test)]
    pub fn with_child_exit(mut self, code: i32) -> Self {
        self.child_exit = Some(code);
        self
    }

    pub fn with_exit(mut self, status: ExitStatus) -> Self {
        self.exit = Some(status);
        self
    }

    pub fn with_diagnostics(mut self, lines: Vec<String>) -> Self {
        self.diagnostics = lines;
        self
    }

    pub fn with_failure(mut self, error: anyhow::Error) -> Self {
        self.failure = Some(error);
        self
    }
}
