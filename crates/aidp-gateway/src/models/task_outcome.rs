use crate::models::enums::native_reason::NativeReason;
use crate::models::enums::native_run_status::NativeRunStatus;
use aidp_core::plan::TaskStatus;

/// What one plan task left behind: its status, token use, and the paths it wrote or misplaced.
#[derive(Debug, Clone, PartialEq)]
pub struct TaskOutcome {
    pub status: TaskStatus,
    pub run_status: NativeRunStatus,
    pub tokens: Option<u64>,
    pub written: Vec<String>,
    pub violations: Vec<String>,
    pub reason: Option<NativeReason>,
}

impl TaskOutcome {
    /// A task that finished cleanly.
    pub fn succeeded(tokens: Option<u64>, written: Vec<String>) -> Self {
        Self {
            status: TaskStatus::Success,
            run_status: NativeRunStatus::Running,
            tokens,
            written,
            violations: Vec::new(),
            reason: None,
        }
    }

    /// A task that failed for `reason`.
    pub fn failed(reason: NativeReason) -> Self {
        Self {
            status: TaskStatus::Failed,
            run_status: NativeRunStatus::Paused,
            tokens: None,
            written: Vec::new(),
            violations: Vec::new(),
            reason: Some(reason),
        }
    }

    /// A task stopped by a cancel.
    pub fn cancelled() -> Self {
        Self {
            status: TaskStatus::Cancelled,
            run_status: NativeRunStatus::Cancelled,
            tokens: None,
            written: Vec::new(),
            violations: Vec::new(),
            reason: None,
        }
    }

    /// True when the task counts as done for its dependents.
    pub fn is_success(&self) -> bool {
        self.status == TaskStatus::Success
    }
}
