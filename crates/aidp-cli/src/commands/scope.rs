use super::run::vault_leak_lines;
use super::run_loader::load_run;
use super::CommandOutput;
use crate::args::ScopeOp;
use crate::ctx::Ctx;
use crate::models::envelopes::OkEnvelope;
use crate::models::scope_check_refusal::ScopeCheckRefusal;
use crate::output::to_spaced_json;
use aidp_core::ExitStatus;
use std::path::PathBuf;

const RUN_ENV: &str = "AIDP_RUN";
const STAGE_ENV: &str = "AIDP_STAGE";

pub fn run(ctx: &Ctx, op: ScopeOp) -> anyhow::Result<CommandOutput> {
    match op {
        ScopeOp::Resolve { run, stage } => resolve(ctx, &run, &stage),
        ScopeOp::Check { path, run, stage } => check(ctx, path, run, stage),
        ScopeOp::Verify { run } => verify(ctx, &run),
    }
}

fn resolve(ctx: &Ctx, run: &str, stage: &str) -> anyhow::Result<CommandOutput> {
    let workspace = ctx.workspace();
    let run_state = load_run(ctx, &workspace, run)?;
    let scope_report = aidp_core::scope::scope_json(&run_state, stage)?;
    aidp_core::scope::write_scope_json(&run_state, stage, &scope_report)?;
    Ok(CommandOutput::text(serde_json::to_string_pretty(
        &scope_report,
    )?))
}

fn refusal(message: String) -> anyhow::Result<CommandOutput> {
    Ok(CommandOutput::text(serde_json::to_string(
        &ScopeCheckRefusal::new(message),
    )?))
}

fn check(
    ctx: &Ctx,
    path: PathBuf,
    run: Option<String>,
    stage: Option<String>,
) -> anyhow::Result<CommandOutput> {
    let workspace = ctx.workspace();
    let run = run
        .or_else(|| std::env::var(RUN_ENV).ok())
        .ok_or_else(|| anyhow::anyhow!("--run or AIDP_RUN required"))?;
    let stage = stage
        .or_else(|| std::env::var(STAGE_ENV).ok())
        .ok_or_else(|| anyhow::anyhow!("--stage or AIDP_STAGE required"))?;
    let run_state = load_run(ctx, &workspace, &run)?;
    let rules = match aidp_core::scope::rules_for(&run_state, &stage) {
        Ok(rules) => rules,
        Err(e) => return Ok(refusal(e.to_string())?.with_failure(e)),
    };
    match rules.check(&path) {
        Ok(()) => Ok(CommandOutput::text(to_spaced_json(&OkEnvelope {
            ok: true,
        })?)),
        Err(e) => Ok(refusal(e.to_string())?.with_exit(ExitStatus::PolicyRefusal)),
    }
}

fn verify(ctx: &Ctx, run: &str) -> anyhow::Result<CommandOutput> {
    let workspace = ctx.workspace();
    let run_state = load_run(ctx, &workspace, run)?;
    let report = aidp_core::scope::verify_run_report(&run_state)?;
    let output = CommandOutput::json(ctx.format(), serde_json::to_value(&report)?);
    if !report.vault_leaks.is_empty() {
        return Ok(output
            .with_diagnostics(vault_leak_lines(&report.vault_leaks))
            .with_exit(ExitStatus::PolicyRefusal));
    }
    if !report.violations.is_empty() {
        return Ok(output.with_exit(ExitStatus::NegativeVerdict));
    }
    Ok(output)
}
