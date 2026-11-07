use serde_json::Value;
use std::collections::HashMap;

use crate::models::mock_injected_failure::InjectedFailure;
use crate::models::mock_recorded_request::RecordedRequest;
use crate::models::mock_reviewer_set::ReviewerSet;

/// Mutable state of the flow-action routes, wiped by the reset route.
#[derive(Default)]
pub struct FlowState {
    pub recorded: Vec<RecordedRequest>,
    pub failures: Vec<InjectedFailure>,
    pub reviewer_posts: u64,
    pub reviewers: HashMap<String, ReviewerSet>,
    pub workflow_files: HashMap<String, String>,
    /// Seeded issues by `owner/repo/number`, as the REST API shows them.
    pub issues: HashMap<String, Value>,
    /// Comments posted on issues, by `owner/repo/number`.
    pub issue_comments: HashMap<String, Vec<String>>,
}

impl FlowState {
    /// Returns the forced status for `url`, consuming one use when bounded.
    pub fn take_failure(&mut self, url: &str) -> Option<u16> {
        let index = self
            .failures
            .iter()
            .position(|failure| url.contains(&failure.path_contains))?;
        let status = self.failures[index].status;
        match self.failures[index].remaining {
            Some(1) => {
                self.failures.remove(index);
            }
            Some(count) => self.failures[index].remaining = Some(count.saturating_sub(1)),
            None => {}
        }
        Some(status)
    }
}
