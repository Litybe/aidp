use serde::{Deserialize, Serialize};

/// Final status of a pipeline run, as the executor reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PipelineRunStatus {
    Done,
    Failed,
    Orphaned,
    Cancelled,
}

impl PipelineRunStatus {
    /// The wire value carried by `Event::PipelineFinished`.
    pub fn as_str(self) -> &'static str {
        match self {
            PipelineRunStatus::Done => "done",
            PipelineRunStatus::Failed => "failed",
            PipelineRunStatus::Orphaned => "orphaned",
            PipelineRunStatus::Cancelled => "cancelled",
        }
    }

    /// Parses a wire value; `None` when it names no known status.
    pub fn parse(value: &str) -> Option<Self> {
        [Self::Done, Self::Failed, Self::Orphaned, Self::Cancelled]
            .into_iter()
            .find(|status| status.as_str() == value)
    }
}
