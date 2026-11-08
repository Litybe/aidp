//! Run status verdict, resume anchors and gate approval.

use super::layout::save;
use super::{RunState, StageState, StageStatus};
use crate::clock::{Clock, SystemClock};
use crate::cost::PriceTable;
use crate::git::GitCli;
use crate::id::SystemIdGen;
use crate::models::enums::run_status::RunStatus;
use crate::models::enums::workflow_status::WorkflowStatus;
use crate::models::run_state::{GateDecision, RunPorts};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// One-word verdict for the whole run, shared by `aidp status`, the summary
/// renderer and the generated Obsidian notes so all three agree.
pub fn overall_status(state: &RunState) -> RunStatus {
    let terminal = |s: &StageState| {
        matches!(
            s.status,
            StageStatus::Success | StageStatus::Cancel | StageStatus::Skipped
        )
    };
    let all_done = state.stages.iter().all(terminal);
    if state.stages.iter().any(|s| s.status == StageStatus::Cancel) && all_done {
        RunStatus::Failed
    } else if all_done {
        RunStatus::Done
    } else if state
        .stages
        .iter()
        .any(|s| s.status == StageStatus::InProgress)
    {
        RunStatus::InProgress
    } else if state.pending_gate.is_some() {
        RunStatus::AwaitingGate
    } else if state
        .stages
        .iter()
        .all(|s| s.status == StageStatus::Pending)
    {
        RunStatus::Pending
    } else {
        RunStatus::Paused
    }
}

/// Mark a stage `Skipped` (its `when:` evaluated false). Terminal, non-failing.
pub fn stage_skip(state: &mut RunState, stage_name: &str) -> anyhow::Result<()> {
    let stage = state
        .stages
        .iter_mut()
        .find(|s| s.name == stage_name)
        .ok_or_else(|| anyhow::anyhow!("stage not found: {stage_name}"))?;
    stage.status = StageStatus::Skipped;
    stage.finished_at = Some(SystemClock.now_rfc3339());
    save(state)?;
    Ok(())
}

/// Mark a stage `Success` without dispatch (e.g. a loop container whose body
/// already completed). No tokens, zero duration.
pub fn stage_complete_noop(state: &mut RunState, stage_name: &str) -> anyhow::Result<()> {
    let stage = state
        .stages
        .iter_mut()
        .find(|s| s.name == stage_name)
        .ok_or_else(|| anyhow::anyhow!("stage not found: {stage_name}"))?;
    stage.status = StageStatus::Success;
    stage.finished_at = Some(SystemClock.now_rfc3339());
    save(state)?;
    Ok(())
}

/// Record the Workflow runId, newly-approved gates and the gate the run is paused at.
pub fn record_resume(
    state: &mut RunState,
    workflow_run_id: Option<String>,
    approved_gates: &[String],
    pending_gate: Option<String>,
    crossed_boundary: Option<String>,
) -> anyhow::Result<()> {
    let clock = SystemClock;
    let ids = SystemIdGen;
    let git = GitCli;
    let prices = PriceTable::from_env_or_embedded();
    let ports = RunPorts {
        clock: &clock,
        ids: &ids,
        git: &git,
        prices: &prices,
    };
    record_resume_with(
        state,
        workflow_run_id,
        approved_gates,
        pending_gate,
        crossed_boundary,
        &ports,
    )
}

/// [`record_resume`] with its clock port injected — see [`RunPorts`].
pub fn record_resume_with(
    state: &mut RunState,
    workflow_run_id: Option<String>,
    approved_gates: &[String],
    pending_gate: Option<String>,
    crossed_boundary: Option<String>,
    ports: &RunPorts,
) -> anyhow::Result<()> {
    if let Some(rid) = workflow_run_id {
        state.workflow_run_id = Some(rid);
    }
    let now = ports.clock.now_rfc3339();
    for g in approved_gates {
        let already = state
            .gates
            .iter()
            .any(|d| &d.after_stage == g && d.decision == GateDecisionKind::Approve);
        if !already {
            state.gates.push(GateDecision {
                after_stage: g.clone(),
                asked_at: Some(now.clone()),
                decision: GateDecisionKind::Approve,
                notes: None,
            });
        }
        if state.pending_gate.as_deref() == Some(g.as_str()) {
            state.pending_gate = None;
        }
    }
    if let Some(pg) = pending_gate {
        let approved = state
            .gates
            .iter()
            .any(|d| d.after_stage == pg && d.decision == GateDecisionKind::Approve);
        state.pending_gate = if approved { None } else { Some(pg) };
    }
    if let Some(cb) = crossed_boundary {
        if !state.crossed_boundaries.iter().any(|c| c == &cb) {
            state.crossed_boundaries.push(cb);
        }
    }
    save(state)?;
    Ok(())
}

/// Stage names of gates approved so far — seeds the emitted `__approved`.
pub fn approved_gates(state: &RunState) -> Vec<String> {
    state
        .gates
        .iter()
        .filter(|g| g.decision == GateDecisionKind::Approve)
        .map(|g| g.after_stage.clone())
        .collect()
}

/// Outcome of a gate: `approve`, `revise` and `abort` are persisted, `pending` is only echoed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateDecisionKind {
    Approve,
    Revise,
    Abort,
    Pending,
}

impl std::fmt::Display for GateDecisionKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl GateDecisionKind {
    /// Wire value, identical to the serialized form.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Approve => "approve",
            Self::Revise => "revise",
            Self::Abort => "abort",
            Self::Pending => "pending",
        }
    }
}

pub const WORKFLOW_RESULT_FILE: &str = "workflow-result.json";

/// Resume anchor recovered from a Workflow result file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeAnchor {
    pub workflow_run_id: Option<String>,
    pub pending_gate: Option<String>,
    pub workflow_status: Option<String>,
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(default)]
struct WorkflowResultBody {
    status: Option<String>,
    gate: Option<String>,
    #[serde(rename = "runId")]
    run_id: Option<String>,
}

#[derive(Debug, Default, serde::Deserialize)]
#[serde(default)]
struct WorkflowResultFile {
    result: Option<WorkflowResultBody>,
    #[serde(rename = "runId")]
    run_id: Option<String>,
    #[serde(flatten)]
    bare: WorkflowResultBody,
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|text| !text.is_empty())
}

/// Parses a Workflow result, bare or wrapped as {"result": {...}}; pending_gate is set only
/// for a non-empty gate on an awaiting_gate status.
pub fn parse_resume_anchor(raw: &str) -> anyhow::Result<ResumeAnchor> {
    let file: WorkflowResultFile = serde_json::from_str(raw)?;
    let outer_run_id = file.run_id;
    let body = file.result.unwrap_or(file.bare);
    let workflow_run_id = non_empty(body.run_id).or_else(|| non_empty(outer_run_id));
    let pending_gate = if body.status.as_deref().map(WorkflowStatus::parse)
        == Some(WorkflowStatus::AwaitingGate)
    {
        non_empty(body.gate)
    } else {
        None
    };
    Ok(ResumeAnchor {
        workflow_run_id,
        pending_gate,
        workflow_status: body.status,
    })
}

/// Reads <run_dir>/workflow-result.json: Ok(None) when missing, an error naming the path when
/// unreadable or malformed.
pub fn read_resume_anchor(run_dir: &Path) -> anyhow::Result<Option<ResumeAnchor>> {
    use anyhow::Context;
    let path = run_dir.join(WORKFLOW_RESULT_FILE);
    let raw = match crate::text::read_text_lenient(&path) {
        Ok(raw) => raw,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(
                anyhow::Error::new(error).context(format!("failed to read {}", path.display()))
            )
        }
    };
    let anchor =
        parse_resume_anchor(&raw).with_context(|| format!("failed to parse {}", path.display()))?;
    Ok(Some(anchor))
}

/// True when the state holds no anchor but the result file names a pending gate.
pub fn anchor_not_persisted(state: &RunState) -> bool {
    if state.workflow_run_id.is_some() || state.pending_gate.is_some() {
        return false;
    }
    matches!(
        read_resume_anchor(&state.run_dir),
        Ok(Some(anchor)) if anchor.pending_gate.is_some()
    )
}

/// A stage DECLARES a gate when `gate:` is present and neither `false` nor `null`.
fn declares_gate(stage: &crate::pipeline::Stage) -> bool {
    match &stage.gate {
        None => false,
        Some(serde_yaml::Value::Bool(b)) => *b,
        Some(serde_yaml::Value::Null) => false,
        Some(_) => true,
    }
}

/// Resolve `--approve-gate` values (repeatable, `*` accepted) against the pipeline's DECLARED
/// gates — a stage whose `gate:` key is present and neither `false` nor `null`.
pub fn resolve_approve_gates(
    pipe: &crate::pipeline::Pipeline,
    requested: &[String],
) -> (Vec<String>, Vec<String>) {
    let declared: Vec<String> = crate::pipeline::flat_stages(pipe)
        .into_iter()
        .filter(|s| declares_gate(s))
        .map(|s| s.name.clone())
        .collect();

    let mut to_approve: Vec<String> = Vec::new();
    let mut unknown: Vec<String> = Vec::new();
    for req in requested {
        if req == "*" {
            for name in &declared {
                if !to_approve.contains(name) {
                    to_approve.push(name.clone());
                }
            }
        } else if declared.contains(req) {
            if !to_approve.contains(req) {
                to_approve.push(req.clone());
            }
        } else if !unknown.contains(req) {
            unknown.push(req.clone());
        }
    }
    (to_approve, unknown)
}

/// Parse the comma-joined gate list the ticket dispatcher writes into
/// `AIDP_AUTO_APPROVE_GATES` (including the single value `"*"`).
pub fn parse_auto_approve_gates(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}
