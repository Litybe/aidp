use std::path::PathBuf;

/// Per-stage write rules: what a single path may touch.
#[derive(Clone)]
pub struct ScopeRules {
    pub mode: String,
    pub stage_dir: PathBuf,
    /// Repo root that `production` and `tests` modes are anchored to.
    /// A relative candidate is resolved against it.
    pub workspace_root: PathBuf,
    pub allowed: Vec<glob::Pattern>,
    pub denied: Vec<glob::Pattern>,
}

/// A changed path that no successful stage's declared scope accepts.
/// `reason` is the rejection text of the last stage checked.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Violation {
    pub path: String,
    pub reason: String,
}

/// Result of `verify_run_report`: violations plus honesty flags.
/// An empty `violations` proves nothing when `union_universal` or `uninspected_stages` is set.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ScopeReport {
    pub violations: Vec<Violation>,
    /// Files that belong under the vault but were written to the repo.
    /// Kept apart from violations because a `repo_wide` scope accepts them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub vault_leaks: Vec<crate::vault_leak::VaultLeak>,
    /// True when a successful stage declared `repo_wide`, so a clean report is not proof.
    pub union_universal: bool,
    /// Successful stages whose changes could not be inspected (worktree isolation).
    pub uninspected_stages: Vec<String>,
}

/// One stage's resolved scope, as printed by `aidp scope resolve`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ResolvedScope {
    pub stage: String,
    pub mode: String,
    pub stage_dir: PathBuf,
    pub allowed: Vec<String>,
    pub denied: Vec<String>,
}
