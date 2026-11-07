use super::progress::{render_run_progress, render_run_view};
use super::run_loader::load_run;
use super::workflow::{clear_run_in_flight, emit_to_disk, in_flight_run_id, tally_task_statuses};
use super::CommandOutput;
use crate::args::RunOp;
use crate::ctx::Ctx;
use crate::models::envelopes::RecordResumeEnvelope;
use crate::models::exec_request::ExecRequest;
use crate::models::exec_summary::ExecSummary;
use crate::models::run_start_envelope::RunStartEnvelope;
use crate::models::vault_location::VaultLocation;
use crate::output::to_spaced_json;
use crate::pipelines::resolve_pipeline_ref;
use aidp_core::run::{INPUT_JIRA_KEY, INPUT_TARGET};
use aidp_core::ExitStatus;
use aidp_gateway::models::enums::native_run_status::NativeRunStatus;
use aidp_gateway::models::native_run_start::StartNativeRun;
use aidp_gateway::native_runs::{NativeDeps, NativeRunManager};
use std::path::{Path, PathBuf};

const HTTP_NOT_FOUND_PREFIX: &str = "HTTP 404";
const HTTP_BAD_REQUEST_PREFIX: &str = "HTTP 400";
const FOREGROUND_TIMEOUT_SECS: u64 = 7 * 24 * 3600;
const EXECUTION_PLAN_FILE: &str = "execution-plan.json";
const STDIN_SOURCE: &str = "-";
const AUTO_APPROVE_GATES_ENV: &str = "AIDP_AUTO_APPROVE_GATES";
const UNKNOWN_WORKFLOW_STATUS: &str = "unknown";
const CLAUDE_PROJECTS_RELATIVE: &str = ".claude/projects";
const VAULT_LEAK_REMEDY: &str = "These paths belong under the configured vault, not the repo. Move them into the vault (or `git rm --cached` them if already committed) and re-run.";

pub fn run(ctx: &Ctx, op: RunOp) -> anyhow::Result<CommandOutput> {
    match op {
        RunOp::Init {
            pipeline,
            target,
            inputs,
        } => init(ctx, &pipeline, target, inputs),
        RunOp::Start {
            pipeline,
            target,
            inputs,
            input,
            approve_gate,
            inputs_file,
            stage_model,
        } => start(
            ctx,
            StartRequest {
                pipeline,
                target,
                inputs,
                input,
                approve_gate,
                inputs_file,
                stage_model,
            },
        ),
        RunOp::Exec {
            template,
            target,
            input,
            inputs_file,
            provider,
            approve_gate,
            foreground,
            url,
        } => exec(
            ctx,
            ExecRequest {
                template,
                target,
                input,
                inputs_file,
                provider,
                approve_gate,
                foreground,
                url,
            },
        ),
        RunOp::State { run } => render_run_progress(ctx, &run),
        RunOp::Summary { run } => summary(ctx, run),
        RunOp::Reconcile { run, from } => reconcile(ctx, &run, &from),
        RunOp::PlanExec { run } => plan_exec(ctx, &run),
        RunOp::RecordResume {
            run,
            workflow_run_id,
            approved_gate,
            pending_gate,
            crossed_boundary,
            from_result,
        } => record_resume(
            ctx,
            &run,
            ResumeRequest {
                workflow_run_id,
                approved_gate,
                pending_gate,
                crossed_boundary,
                from_result,
            },
        ),
        RunOp::StageTokens {
            session_id,
            agent_id,
            claude_projects_dir,
        } => stage_tokens(ctx, &session_id, &agent_id, claude_projects_dir),
        RunOp::Verify { run, stage } => verify(ctx, &run, stage),
    }
}

/// Parses `--inputs`, the JSON object handed to `run init`.
fn parse_inputs_flag(inputs: Option<String>) -> anyhow::Result<serde_json::Value> {
    match inputs {
        Some(s) => {
            serde_json::from_str(&s).map_err(|e| anyhow::anyhow!("--inputs invalid JSON: {e}"))
        }
        None => Ok(serde_json::json!({})),
    }
}

/// Seeds `target` and `jira_key` from the positional target without overriding explicit inputs.
fn seed_target_inputs(inputs_json: &mut serde_json::Value, target: Option<String>) {
    let Some(target) = target else { return };
    if let serde_json::Value::Object(map) = inputs_json {
        let val = serde_json::Value::String(target);
        map.entry(INPUT_TARGET.to_string())
            .or_insert_with(|| val.clone());
        map.entry(INPUT_JIRA_KEY.to_string()).or_insert_with(|| val);
    }
}

fn init(
    ctx: &Ctx,
    pipeline: &str,
    target: Option<String>,
    inputs: Option<String>,
) -> anyhow::Result<CommandOutput> {
    let workspace = ctx.workspace();
    let pipeline = resolve_pipeline_ref(ctx, &workspace, pipeline)?;
    let mut inputs_json = parse_inputs_flag(inputs)?;
    seed_target_inputs(&mut inputs_json, target);
    let resolved = ctx.vault_override();
    let mut run_state =
        aidp_core::run::init(&pipeline, &workspace, inputs_json, resolved.as_ref())?;
    freeze_model_policy(&mut run_state, resolved.as_ref(), &[])?;
    let mut diagnostics = vec![format!("run_dir: {}", run_state.run_dir.display())];
    if let Some(resolved_vault) = &resolved {
        diagnostics.push(format!(
            "vault:   {} (via {})",
            resolved_vault.root.display(),
            resolved_vault.source.as_str()
        ));
        if let Some(a) = &resolved_vault.ambiguity {
            diagnostics.push(format!("warning: {a}"));
        }
    }
    Ok(CommandOutput::text(run_state.run_id.clone()).with_diagnostics(diagnostics))
}

struct StartRequest {
    pipeline: String,
    target: Option<String>,
    inputs: Option<String>,
    input: Vec<(String, String)>,
    approve_gate: Vec<String>,
    inputs_file: Option<PathBuf>,
    stage_model: Vec<String>,
}

/// Freezes the model policy of the vault and the `--stage-model` overrides into the run.
fn freeze_model_policy(
    run_state: &mut aidp_core::run::RunState,
    vault: Option<&aidp_core::vault::Vault>,
    stage_model: &[String],
) -> anyhow::Result<()> {
    let cli = aidp_core::model_policy::parse_cli_overrides(stage_model)
        .map_err(|message| anyhow::anyhow!(message))?;
    let vault_root = vault.map(|resolved| resolved.root.as_path());
    aidp_core::model_policy::apply(run_state, vault_root, &cli);
    aidp_core::run::save(run_state)
}

/// Builds the native launch request of `run exec`, registering the workspace as a project.
/// The workspace and the vault are made absolute: the daemon does not share this current directory.
fn exec_launch(ctx: &Ctx, request: &ExecRequest) -> anyhow::Result<StartNativeRun> {
    let workspace = ctx.absolute(&ctx.workspace());
    resolve_pipeline_ref(ctx, &workspace, &request.template)?;
    let mut inputs_json = start_inputs(None, &request.inputs_file)?;
    let serde_json::Value::Object(map) = &mut inputs_json else {
        anyhow::bail!("--inputs-file must hold a JSON object");
    };
    apply_key_value_inputs(map, &request.input)?;
    let project = aidp_gateway::projects::Projects::load(&aidp_gateway::paths::state_dir()?)
        .add(&workspace.display().to_string());
    Ok(StartNativeRun {
        client_request_id: None,
        project_id: project.id,
        template_id: request.template.clone(),
        inputs: map.clone().into_iter().collect(),
        target: request.target.clone(),
        workspace: Some(workspace),
        provider: request.provider.clone(),
        autonomous: false,
        auto_approve_gates: requested_gates(request.approve_gate.clone()),
        vault: ctx.vault_flag().map(|vault| ctx.absolute(vault)),
        no_vault: ctx.globals.no_vault,
        ..StartNativeRun::default()
    })
}

/// `aidp run exec`: starts a native run through the daemon, or drives it in-process.
fn exec(ctx: &Ctx, request: ExecRequest) -> anyhow::Result<CommandOutput> {
    let launch = exec_launch(ctx, &request)?;
    if request.foreground {
        return exec_foreground(launch);
    }
    let body = serde_json::to_value(&launch)?;
    let answer = aidp_gateway::client::start_native_run(&request.url, body).map_err(|error| {
        let message = error.to_string();
        let status = if message.starts_with(HTTP_NOT_FOUND_PREFIX) {
            ExitStatus::NotFound
        } else if message.starts_with(HTTP_BAD_REQUEST_PREFIX) {
            ExitStatus::Usage
        } else {
            ExitStatus::UpstreamFailure
        };
        anyhow::Error::from(aidp_core::TypedExit { status, message })
    })?;
    Ok(CommandOutput::text(serde_json::to_string(&answer)?))
}

/// Runs the native executor in this process until the run settles; exit 0 only when it is done.
fn exec_foreground(launch: StartNativeRun) -> anyhow::Result<CommandOutput> {
    let state_dir = aidp_gateway::paths::state_dir()?;
    let bus = std::sync::Arc::new(aidp_gateway::bus::Bus::new());
    let projects = std::sync::Arc::new(aidp_gateway::projects::Projects::load(&state_dir));
    let sessions =
        aidp_gateway::sessions::SessionManager::load(&state_dir, bus.clone(), projects.clone());
    let deps = NativeDeps::new(
        sessions,
        projects,
        bus,
        std::sync::Arc::new(aidp_gateway::flows::registry::AlwaysAllow),
        std::sync::Arc::new(aidp_gateway::models::local_lock::LocalLock::default()),
        &state_dir,
        aidp_gateway::paths::skill_root(),
    );
    let manager = NativeRunManager::new(deps);
    let run_id = manager.start(launch)?;
    let timeout = std::time::Duration::from_secs(FOREGROUND_TIMEOUT_SECS);
    let view = manager
        .wait_for(&run_id, timeout)
        .ok_or_else(|| anyhow::anyhow!("run {run_id} vanished"))?;
    let summary = ExecSummary {
        run_id,
        status: view.status,
        reason: view.reason,
    };
    let status = if view.status == NativeRunStatus::Done {
        ExitStatus::Success
    } else {
        ExitStatus::NegativeVerdict
    };
    Ok(CommandOutput::text(serde_json::to_string(&summary)?).with_exit(status))
}

/// The inputs JSON of `run start`: `--inputs-file`, else `--inputs`, else empty.
fn start_inputs(
    inputs: Option<String>,
    inputs_file: &Option<PathBuf>,
) -> anyhow::Result<serde_json::Value> {
    match (inputs, inputs_file) {
        (_, Some(p)) => {
            let s = std::fs::read_to_string(p)
                .map_err(|e| anyhow::anyhow!("--inputs-file {}: {e}", p.display()))?;
            serde_json::from_str(&s)
                .map_err(|e| anyhow::anyhow!("--inputs-file {} invalid JSON: {e}", p.display()))
        }
        (Some(s), None) => {
            serde_json::from_str(&s).map_err(|e| anyhow::anyhow!("--inputs invalid JSON: {e}"))
        }
        (None, None) => Ok(serde_json::json!({})),
    }
}

/// Folds repeatable `--input key=value` pairs into the inputs object.
/// A key given more than once accumulates into a JSON array, for `kind: list` inputs.
fn apply_key_value_inputs(
    map: &mut serde_json::Map<String, serde_json::Value>,
    input: &[(String, String)],
) -> anyhow::Result<()> {
    for (key, value) in input {
        let val = serde_json::Value::String(value.to_string());
        match map.get_mut(key.as_str()) {
            Some(serde_json::Value::Array(arr)) => arr.push(val),
            Some(existing) => {
                let prev = existing.clone();
                *existing = serde_json::Value::Array(vec![prev, val]);
            }
            None => {
                map.insert(key.to_string(), val);
            }
        }
    }
    Ok(())
}

/// Gates to pre-approve: the flag when given, else the comma-separated environment variable.
fn requested_gates(approve_gate: Vec<String>) -> Vec<String> {
    if !approve_gate.is_empty() {
        return approve_gate;
    }
    std::env::var(AUTO_APPROVE_GATES_ENV)
        .ok()
        .map(|raw| aidp_core::run::parse_auto_approve_gates(&raw))
        .unwrap_or_default()
}

/// Bakes the requested gate approvals into the run before its first emit.
/// Returns the approved gates; unknown names become warnings in `diagnostics`.
fn pre_approve_gates(
    run_state: &mut aidp_core::run::RunState,
    requested: &[String],
    diagnostics: &mut Vec<String>,
) -> anyhow::Result<Vec<String>> {
    if requested.is_empty() {
        return Ok(Vec::new());
    }
    let pipe = aidp_core::pipeline::load(&run_state.pipeline_path)?;
    let (to_approve, unknown) = aidp_core::run::resolve_approve_gates(&pipe, requested);
    for u in &unknown {
        diagnostics.push(format!(
            "warning: --approve-gate {u}: no stage declares a gate by that name in this pipeline; ignored"
        ));
    }
    if !to_approve.is_empty() {
        aidp_core::run::record_resume(run_state, None, &to_approve, None, None)?;
    }
    Ok(to_approve)
}

fn start(ctx: &Ctx, request: StartRequest) -> anyhow::Result<CommandOutput> {
    let workspace = ctx.workspace();
    let pipeline_path = resolve_pipeline_ref(ctx, &workspace, &request.pipeline)?;
    let mut inputs_json = start_inputs(request.inputs, &request.inputs_file)?;
    if let serde_json::Value::Object(map) = &mut inputs_json {
        apply_key_value_inputs(map, &request.input)?;
    }
    seed_target_inputs(&mut inputs_json, request.target);
    let resolved = ctx.vault_override();
    let mut run_state =
        aidp_core::run::init(&pipeline_path, &workspace, inputs_json, resolved.as_ref())?;
    freeze_model_policy(&mut run_state, resolved.as_ref(), &request.stage_model)?;
    let requested = requested_gates(request.approve_gate);
    let mut diagnostics = Vec::new();
    let auto_approved = match pre_approve_gates(&mut run_state, &requested, &mut diagnostics) {
        Ok(approved) => approved,
        Err(e) => {
            return Ok(CommandOutput::empty()
                .with_diagnostics(diagnostics)
                .with_failure(e))
        }
    };
    let (script_path, emitted) = match emit_to_disk(ctx, &workspace, &mut run_state, None) {
        Ok(emitted) => emitted,
        Err(e) => {
            return Ok(CommandOutput::empty()
                .with_diagnostics(diagnostics)
                .with_failure(e))
        }
    };
    let already: std::collections::HashSet<String> = aidp_core::run::approved_gates(&run_state)
        .into_iter()
        .collect();
    let remaining: Vec<String> = emitted
        .gates
        .iter()
        .filter(|g| !already.contains(g.as_str()))
        .cloned()
        .collect();
    let envelope = RunStartEnvelope {
        auto_approved_gates: &auto_approved,
        awaiting_composite: emitted.awaiting_composite.is_some(),
        awaiting_plan: emitted.awaiting_plan.is_some(),
        gates: &remaining,
        last_workflow_status: &run_state.last_workflow_status,
        run_dir: run_state.run_dir.display().to_string(),
        run_id: &run_state.run_id,
        script_path: script_path.display().to_string(),
        vault: resolved.as_ref().map(|resolved_vault| VaultLocation {
            root: resolved_vault.root.display().to_string(),
            source: resolved_vault.source.as_str(),
        }),
        warnings: run_state
            .init_warnings
            .iter()
            .chain(emitted.warnings.iter())
            .cloned()
            .collect(),
    };
    Ok(CommandOutput::text(serde_json::to_string(&envelope)?).with_diagnostics(diagnostics))
}

fn summary(ctx: &Ctx, run: Option<String>) -> anyhow::Result<CommandOutput> {
    let workspace = ctx.workspace();
    let Some(run) = run.or_else(|| in_flight_run_id(&workspace)) else {
        return Ok(CommandOutput::empty());
    };
    let run_state = load_run(ctx, &workspace, &run)?;
    let mut text = aidp_core::run::render_summary(&run_state);
    if let Some(line) = plan_task_status_line(&run_state) {
        text.push('\n');
        text.push_str(&line);
    }
    Ok(render_run_view(ctx.format(), &run_state, text))
}

/// `- plan tasks by status: cancelled: 1, done: 3`, or `None` when no plan task exists.
/// `cancelled` counts apart from `failed`: a never-dispatched task did not error.
fn plan_task_status_line(run_state: &aidp_core::run::RunState) -> Option<String> {
    if run_state.plan_schedules.is_empty() {
        return None;
    }
    let counts = tally_task_statuses(
        run_state
            .plan_schedules
            .values()
            .flat_map(|s| s.tasks.iter()),
    );
    if counts.is_empty() {
        return None;
    }
    let parts: Vec<String> = counts
        .iter()
        .map(|(key, count)| format!("{key}: {count}"))
        .collect();
    Some(format!("- plan tasks by status: {}", parts.join(", ")))
}

/// The Workflow result JSON, from a file or from stdin when `from` is `-`.
fn read_result_source(from: &str) -> anyhow::Result<serde_json::Value> {
    let text = if from == STDIN_SOURCE {
        use std::io::Read as _;
        let mut s = String::new();
        std::io::stdin()
            .read_to_string(&mut s)
            .map_err(|e| anyhow::anyhow!("read stdin: {e}"))?;
        s
    } else {
        std::fs::read_to_string(from).map_err(|e| anyhow::anyhow!("read {from}: {e}"))?
    };
    serde_json::from_str(&text).map_err(|e| anyhow::anyhow!("parse {from}: {e}"))
}

/// The stderr lines reporting vault artefacts written into the repo tree.
pub fn vault_leak_lines(leaks: &[aidp_core::vault_leak::VaultLeak]) -> Vec<String> {
    let mut lines = vec![format!(
        "error: {} vault artefact(s) were written into the repo tree instead of the vault:",
        leaks.len()
    )];
    for leak in leaks {
        lines.push(format!(
            "  {} ({:?}): {}",
            leak.path, leak.kind, leak.reason
        ));
    }
    lines.push(VAULT_LEAK_REMEDY.to_string());
    lines
}

/// Compiles the run's typed execution plan and writes it pretty-printed into the run dir.
fn plan_exec(ctx: &Ctx, run: &str) -> anyhow::Result<CommandOutput> {
    let workspace = ctx.workspace();
    let run_state = load_run(ctx, &workspace, run)?;
    let pipe = aidp_core::pipeline::load(&run_state.pipeline_path)?;
    let plan = aidp_core::exec_plan::compile_exec_plan(
        &pipe,
        &run_state,
        &aidp_core::workflow_emit::DiskPlanSource,
    )?;
    let path = run_state.run_dir.join(EXECUTION_PLAN_FILE);
    std::fs::write(&path, serde_json::to_vec_pretty(&plan)?)
        .map_err(|e| anyhow::anyhow!("write {}: {e}", path.display()))?;
    let absolute = std::fs::canonicalize(&path)?;
    Ok(CommandOutput::text(absolute.display().to_string()))
}

fn reconcile(ctx: &Ctx, run: &str, from: &str) -> anyhow::Result<CommandOutput> {
    let workspace = ctx.workspace();
    let mut run_state = load_run(ctx, &workspace, run)?;
    let result = read_result_source(from)?;
    let anomalies = aidp_core::run::reconcile(&mut run_state, &result)?;
    clear_run_in_flight(&workspace);
    let mut diagnostics: Vec<String> = anomalies.iter().map(|a| format!("warning: {a}")).collect();
    let output = render_run_view(
        ctx.format(),
        &run_state,
        aidp_core::run::render_summary(&run_state),
    );
    if !run_state.vault_leaks.is_empty() {
        diagnostics.extend(vault_leak_lines(&run_state.vault_leaks));
        return Ok(output
            .with_diagnostics(diagnostics)
            .with_exit(ExitStatus::PolicyRefusal));
    }
    Ok(output.with_diagnostics(diagnostics))
}

struct ResumeRequest {
    workflow_run_id: Option<String>,
    approved_gate: Vec<String>,
    pending_gate: Option<String>,
    crossed_boundary: Option<String>,
    from_result: Option<PathBuf>,
}

/// Merge the anchor stored in a Workflow result file with the explicit flags, which win.
/// A file without a gate still yields its runId; a missing or malformed file is an error.
fn merge_result_anchor(
    path: &Path,
    workflow_run_id: Option<String>,
    pending_gate: Option<String>,
    diagnostics: &mut Vec<String>,
) -> anyhow::Result<(Option<String>, Option<String>)> {
    use anyhow::Context;
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
    let anchor = aidp_core::run::parse_resume_anchor(&raw)
        .with_context(|| format!("failed to parse {}", path.display()))?;
    if anchor.pending_gate.is_none() {
        diagnostics.push(format!(
            "warning: no gate recovered from {} (status: {})",
            path.display(),
            anchor
                .workflow_status
                .as_deref()
                .unwrap_or(UNKNOWN_WORKFLOW_STATUS)
        ));
    }
    Ok((
        workflow_run_id.or(anchor.workflow_run_id),
        pending_gate.or(anchor.pending_gate),
    ))
}

fn record_resume(ctx: &Ctx, run: &str, request: ResumeRequest) -> anyhow::Result<CommandOutput> {
    let mut run_state = load_run(ctx, &ctx.workspace(), run)?;
    let mut diagnostics = Vec::new();
    let (workflow_run_id, pending_gate) = match request.from_result {
        Some(path) => merge_result_anchor(
            &path,
            request.workflow_run_id,
            request.pending_gate,
            &mut diagnostics,
        )?,
        None => (request.workflow_run_id, request.pending_gate),
    };
    aidp_core::run::record_resume(
        &mut run_state,
        workflow_run_id,
        &request.approved_gate,
        pending_gate,
        request.crossed_boundary,
    )?;
    let envelope = RecordResumeEnvelope {
        ok: true,
        workflow_run_id: &run_state.workflow_run_id,
        approved_gates: aidp_core::run::approved_gates(&run_state),
        pending_gate: &run_state.pending_gate,
        crossed_boundaries: &run_state.crossed_boundaries,
    };
    Ok(CommandOutput::text(to_spaced_json(&envelope)?).with_diagnostics(diagnostics))
}

fn stage_tokens(
    ctx: &Ctx,
    session_id: &str,
    agent_id: &str,
    claude_projects_dir: Option<PathBuf>,
) -> anyhow::Result<CommandOutput> {
    let projects_dir = match claude_projects_dir {
        Some(p) => p,
        None => ctx
            .home
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("cannot determine home directory"))?
            .join(CLAUDE_PROJECTS_RELATIVE),
    };
    let jsonl = aidp_core::tokens::find_agent_jsonl(&projects_dir, session_id, agent_id)?;
    let usage = aidp_core::tokens::extract_from_agent_jsonl(&jsonl)?;
    Ok(CommandOutput::text(serde_json::to_string_pretty(&usage)?))
}

fn verify(ctx: &Ctx, run: &str, stage: Option<String>) -> anyhow::Result<CommandOutput> {
    let run_state = load_run(ctx, &ctx.workspace(), run)?;
    let verdicts: Vec<_> = run_state
        .stages
        .iter()
        .filter(|s| stage.as_deref().map(|n| n == s.name).unwrap_or(true))
        .flat_map(aidp_core::verify::verify_outputs)
        .collect();
    Ok(CommandOutput::json(
        ctx.format(),
        serde_json::to_value(&verdicts)?,
    ))
}

#[cfg(test)]
#[path = "../../tests/unit/run_tests.rs"]
mod run_tests;
