//! Persisted run state types (`run-state.json`).

use crate::clock::Clock;
use crate::cost::PriceTable;
use crate::git::Git;
use crate::id::IdGen;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;

/// The clock, id, git and pricing ports the `*_with` run functions read.
pub struct RunPorts<'a> {
    pub clock: &'a dyn Clock,
    pub ids: &'a dyn IdGen,
    pub git: &'a dyn Git,
    pub prices: &'a PriceTable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum StageStatus {
    #[default]
    Pending,
    InProgress,
    Success,
    Cancel,
    /// Not run because its `when:` was false; terminal, non-failing.
    Skipped,
}

impl fmt::Display for StageStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pending => write!(f, "pending"),
            Self::InProgress => write!(f, "in_progress"),
            Self::Success => write!(f, "success"),
            Self::Cancel => write!(f, "cancel"),
            Self::Skipped => write!(f, "skipped"),
        }
    }
}

impl<'de> Deserialize<'de> for StageStatus {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Ok(match raw.as_str() {
            "pending" => Self::Pending,
            "in_progress" | "running" => Self::InProgress,
            "success" | "done" | "completed" => Self::Success,
            "cancel" | "failed" | "blocked" => Self::Cancel,
            "skipped" => Self::Skipped,
            other => {
                return Err(serde::de::Error::unknown_variant(
                    other,
                    &["pending", "in_progress", "success", "cancel", "skipped"],
                ))
            }
        })
    }
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct RunState {
    pub run_id: String,
    pub pipeline_path: PathBuf,
    pub pipeline_id: String,
    pub workspace_root: PathBuf,
    pub run_dir: PathBuf,
    pub started_at: String,
    #[serde(default)]
    pub inputs: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub plan: Vec<Vec<String>>,
    pub stages: Vec<StageState>,
    /// Per-loop iteration state, keyed by loop stage name.
    #[serde(default)]
    pub loops: BTreeMap<String, LoopState>,
    /// Knowledge graph snapshot built at init; `None` when none was built.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knowledge_graph: Option<crate::kg::KnowledgeGraphState>,
    /// The Workflow-tool `runId` used for `resumeFromRunId`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow_run_id: Option<String>,
    /// Gate decisions recorded across resumes, baked into `__approved`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gates: Vec<GateDecision>,
    /// The gate awaiting a human decision, re-proposed on resume; cleared on approval.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_gate: Option<String>,
    /// Composite boundaries already crossed; later emits compile straight through them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub crossed_boundaries: Vec<String>,
    /// Raw `status` of the last Workflow result folded in by `reconcile`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_workflow_status: Option<String>,
    /// Compiled schedule of each `plan_tasks:` stage, keyed by stage name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub plan_schedules: BTreeMap<String, crate::plan::ScheduleState>,
    /// The vault the run started in; `None` means the legacy in-repo layout.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vault_root: Option<PathBuf>,
    /// The repo segment of the vault path, derived from the git remote.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
    /// The per-project knowledge directory, persisted so it cannot shift mid-run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knowledge_root: Option<PathBuf>,
    /// The feature segment that `ticket_root()` points at.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub feature: Option<String>,
    /// The run's total cost, attached best-effort at the end of `reconcile`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost: Option<RunCost>,
    /// The workspace `git rev-parse HEAD` at init, when it is a git repository.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_sha: Option<String>,
    /// The workspace HEAD observed at the end of `reconcile`, best-effort.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_sha_end: Option<String>,
    /// The branch found checked out at init, best-effort.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// Bounded, most-recent-first commit shas observed at the end of `reconcile`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commits: Vec<String>,
    /// Result of `scope::verify_run` at the end of `reconcile`; never blocking.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub scope_violations: Vec<crate::scope::Violation>,
    /// Run artefacts written into the repo tree instead of the vault.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub vault_leaks: Vec<crate::vault_leak::VaultLeak>,
    /// Guard findings accumulated by the emitted script at stage boundaries.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub guard_findings: Vec<serde_json::Value>,
    /// Frozen project knowledge read, reused by every re-emit so prompts stay stable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knowledge_snapshot: Option<crate::knowledge::KnowledgeSnapshot>,
    /// Warnings collected at init time, surfaced on the run.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub init_warnings: Vec<String>,
    /// Execution mode, `workflow` or `native`; `None` for a Workflow-driven run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// True when the run is autonomous: stages never ask, they journal assumptions.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub autonomous: bool,
    /// Choices autonomous stages took in place of a question.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assumptions: Vec<crate::models::assumption::Assumption>,
    /// Decisions stages escalated to a human through `signals.escalations`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub escalations: Vec<crate::models::escalation::Escalation>,
    /// Stage name to model tier fixed at init by the model policy; wins over the template's `model:`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub model_overrides: BTreeMap<String, crate::models::enums::ModelTier>,
    /// Knowledge block frozen per stage at compile time; an empty text means nothing was relevant.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub stage_knowledge: BTreeMap<String, String>,
}

/// Run-level rollup of [`StageUsage`], attached best-effort by `reconcile`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct RunCost {
    pub total_usd: Option<f64>,
    pub tokens_billable: u64,
    pub pricing_version: Option<String>,
    pub captured_at: String,
    #[serde(default)]
    pub by_model: BTreeMap<String, StageUsage>,
}

/// Per-stage usage attributed from a transcript ledger, not from self-reported metrics.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct StageUsage {
    pub input: u64,
    pub output: u64,
    pub cache_creation: u64,
    pub cache_read: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_pinned: Option<String>,
    pub bucket: crate::run::ComplexityBucket,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing_version: Option<String>,
    /// Where the counters came from: jsonl, budget or unavailable.
    pub source: crate::run::UsageSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<u32>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GateDecision {
    pub after_stage: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asked_at: Option<String>,
    /// Recorded outcome of the gate.
    pub decision: crate::run::GateDecisionKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct LoopState {
    /// Current iteration, 1-based; 0 means the loop has not started.
    #[serde(default)]
    pub iter: u32,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct StageState {
    pub name: String,
    pub status: StageStatus,
    #[serde(default)]
    pub started_at: Option<String>,
    #[serde(default)]
    pub finished_at: Option<String>,
    #[serde(default)]
    pub duration_ms: Option<u64>,
    #[serde(default)]
    pub tokens_used: Option<u64>,
    pub stage_dir: PathBuf,
    #[serde(default)]
    pub outputs: BTreeMap<String, PathBuf>,
    /// The sub-agent's summary, folded in by `reconcile`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// The sub-agent's `follow_ups`, folded in by `reconcile`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub follow_ups: Vec<String>,
    /// Declared skills the sub-agent did not load, folded in by `reconcile`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skills_not_loaded: Vec<String>,
    /// Ledger-attributed usage, attached best-effort at the end of `reconcile`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<StageUsage>,
    /// Syntactic findings on the declared outputs; annotation only, never blocking.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub verification: Vec<String>,
}
