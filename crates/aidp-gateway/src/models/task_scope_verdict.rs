/// Paths a plan task wrote outside its own files, its extras or onto a shared file.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TaskScopeVerdict {
    pub violations: Vec<String>,
}

impl TaskScopeVerdict {
    /// True when the task wrote only what it owns.
    pub fn is_clean(&self) -> bool {
        self.violations.is_empty()
    }
}
