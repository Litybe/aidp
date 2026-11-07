use std::time::Duration;

use aidp_agents::{AgentEvent, TokenUsage};

pub use crate::models::enums::fake_step::FakeStep;
pub use crate::models::fake_script::FakeScript;

impl FakeScript {
    /// Starts an empty script that exits with code 0.
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends an arbitrary event.
    pub fn emit(mut self, event: AgentEvent) -> Self {
        self.steps.push(FakeStep::Emit(event));
        self
    }

    /// Appends a text turn.
    pub fn text(self, text: &str) -> Self {
        self.emit(AgentEvent::Text {
            text: text.to_string(),
        })
    }

    /// Appends a usage report.
    pub fn usage(self, usage: TokenUsage) -> Self {
        self.emit(AgentEvent::Usage(usage))
    }

    /// Appends a file write, relative to the spec's work dir.
    pub fn write_file(mut self, path: &str, contents: &str) -> Self {
        self.steps.push(FakeStep::WriteFile {
            path: path.to_string(),
            contents: contents.to_string(),
        });
        self
    }

    /// Appends a pause.
    pub fn delay(mut self, duration: Duration) -> Self {
        self.steps.push(FakeStep::Delay(duration));
        self
    }

    /// Sets the final exit code.
    pub fn exit_code(mut self, code: i32) -> Self {
        self.exit_code = code;
        self
    }
}
