use super::run_loader::{load_run, unknown_gate_refusal};
use super::workflow::emit_to_disk;
use super::CommandOutput;
use crate::args::GateOp;
use crate::ctx::Ctx;
use crate::models::approved_gates_args::ApprovedGatesArgs;
use crate::models::envelopes::GateDecisionEnvelope;
use crate::models::gate_approve_resume::GateApproveResume;
use aidp_core::run::GateDecisionKind;

pub fn run(ctx: &Ctx, op: GateOp) -> anyhow::Result<CommandOutput> {
    match op {
        GateOp::Approve {
            run,
            gate,
            workflow_run_id,
        } => approve(ctx, &run, &gate, workflow_run_id),
        GateOp::Abort { run, gate } => abort(ctx, &run, &gate),
        GateOp::Pending {
            run,
            gate,
            workflow_run_id,
        } => pending(ctx, &run, &gate, workflow_run_id),
    }
}

fn approve(
    ctx: &Ctx,
    run: &str,
    gate: &str,
    workflow_run_id: Option<String>,
) -> anyhow::Result<CommandOutput> {
    let workspace = ctx.workspace();
    let mut run_state = load_run(ctx, &workspace, run)?;
    if let Some(refusal) = unknown_gate_refusal(&run_state, gate) {
        return Ok(refusal);
    }
    aidp_core::run::record_resume(
        &mut run_state,
        workflow_run_id,
        &[gate.to_string()],
        None,
        None,
    )?;
    let gate_name = gate.to_string();
    let (script_path, _emitted) = emit_to_disk(ctx, &workspace, &mut run_state, None)?;
    let envelope = GateApproveResume {
        args: ApprovedGatesArgs {
            approved_gates: [&gate_name],
        },
        resume_from_run_id: &run_state.workflow_run_id,
        script_path: script_path.display().to_string(),
    };
    Ok(CommandOutput::text(serde_json::to_string(&envelope)?))
}

fn abort(ctx: &Ctx, run: &str, gate: &str) -> anyhow::Result<CommandOutput> {
    let workspace = ctx.workspace();
    let mut run_state = load_run(ctx, &workspace, run)?;
    if let Some(refusal) = unknown_gate_refusal(&run_state, gate) {
        return Ok(refusal);
    }
    run_state.gates.push(aidp_core::run::GateDecision {
        after_stage: gate.to_string(),
        asked_at: Some(ctx.clock.now_rfc3339()),
        decision: GateDecisionKind::Abort,
        notes: None,
    });
    if run_state.pending_gate.as_deref() == Some(gate) {
        run_state.pending_gate = None;
    }
    aidp_core::run::save(&run_state)?;
    decision_output(gate, GateDecisionKind::Abort)
}

fn pending(
    ctx: &Ctx,
    run: &str,
    gate: &str,
    workflow_run_id: Option<String>,
) -> anyhow::Result<CommandOutput> {
    let workspace = ctx.workspace();
    let mut run_state = load_run(ctx, &workspace, run)?;
    if let Some(refusal) = unknown_gate_refusal(&run_state, gate) {
        return Ok(refusal);
    }
    aidp_core::run::record_resume(
        &mut run_state,
        workflow_run_id,
        &[],
        Some(gate.to_string()),
        None,
    )?;
    decision_output(gate, GateDecisionKind::Pending)
}

fn decision_output(gate: &str, decision: GateDecisionKind) -> anyhow::Result<CommandOutput> {
    let envelope = GateDecisionEnvelope {
        ok: true,
        gate,
        decision,
    };
    Ok(CommandOutput::text(crate::output::to_spaced_json(
        &envelope,
    )?))
}
