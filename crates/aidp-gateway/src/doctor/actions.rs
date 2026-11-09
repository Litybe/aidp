//! The allowed actions, executed by the daemon after it checked their preconditions.
//! None merges, approves, force-pushes, touches the game or the harness, or opens Unity.

use crate::flows::executor::step_at_cursor;
use crate::flows::facts;
use crate::flows::registry::{FlowAction, SubprocessAction};
use crate::flows::FlowEngine;
use crate::models::constants::doctor::{
    AIDP_LABEL, AIDP_TICKET, DOCTOR_ACTOR, GH_PROGRAM, GH_REPO_FLAG, GH_UPDATE_BRANCH, PARAM_PR,
    PARAM_REPO, PARAM_RESOURCE, PARAM_STEP, REBASE_PARAMS, STATE_READY, STATE_TO_TEST,
};
use crate::models::constants::flow_actions::ACTION_GH_MERGE;
use crate::models::enums::{DoctorAction, DoctorActionResult};
use crate::models::{ActionContext, DoctorDiagnosis, DoctorIncident};
use crate::unity_lock::resource_key;
use aidp_core::process_runner::ProcessSpec;
use aidp_flows::models::constants::resource::UNITY_EDITOR;
use aidp_flows::{ActionOutcome, StepKind, StepStatus};
use serde_json::Value;
use std::collections::BTreeMap;
use std::io::Read;
use std::sync::Arc;
use std::time::{Duration, Instant};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(120);
const COMMAND_POLL: Duration = Duration::from_millis(50);
const KEY_PLACEHOLDER: &str = "{key}";

/// What an action needs.
pub struct ActionTarget<'a> {
    pub engine: &'a Arc<FlowEngine>,
    pub incident: &'a DoctorIncident,
    pub diagnosis: &'a DoctorDiagnosis,
}

use DoctorActionResult::{Done, Failed, Refused};

/// Runs `action` once its precondition holds.
pub fn execute(target: &ActionTarget<'_>, action: DoctorAction) -> DoctorActionResult {
    match action {
        DoctorAction::RetryStep => retry_step(target),
        DoctorAction::RequeueToTest => requeue(target, STATE_TO_TEST),
        DoctorAction::RequeueDev => requeue(target, STATE_READY),
        DoctorAction::AbortDeadFlowAndRequeue => abort_dead_flow(target),
        DoctorAction::UpdatePrBranch => update_pr_branch(target),
        DoctorAction::ReleaseUnityLock => release_unity_lock(target),
        DoctorAction::None => Done("nothing to do".to_string()),
    }
}

fn param<'a>(target: &'a ActionTarget<'_>, name: &str) -> Option<&'a Value> {
    target
        .diagnosis
        .params
        .get(name)
        .filter(|value| !value.is_null())
}

fn text_param(target: &ActionTarget<'_>, name: &str) -> Option<String> {
    param(target, name).and_then(|value| match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        _ => None,
    })
}

/// Retries the step of the incident (or the one named), never a human step. A step still
/// running is reconciled first, so a run that ended is read before the retry.
fn retry_step(target: &ActionTarget<'_>) -> DoctorActionResult {
    let key = target.incident.candidate.key.as_str();
    let Some(step) =
        text_param(target, PARAM_STEP).or_else(|| target.incident.candidate.step.clone())
    else {
        return Refused("no step to retry".to_string());
    };
    let executor = target.engine.executor();
    let Ok((_, state)) = executor.load(key) else {
        return Refused(format!("no flow run for `{key}`"));
    };
    if state.current.as_deref() != Some(step.as_str()) {
        return Refused(format!("`{step}` is not the current step of `{key}`"));
    }
    let definition = step_at_cursor(&state);
    if definition
        .as_ref()
        .is_some_and(|definition| definition.kind == StepKind::Human)
    {
        return Refused(format!(
            "`{step}` is a human step: the doctor never decides one"
        ));
    }
    if definition
        .as_ref()
        .and_then(|definition| definition.use_.as_deref())
        == Some(ACTION_GH_MERGE)
    {
        return Refused(format!(
            "`{step}` merges a pull request: the doctor never merges"
        ));
    }
    if state.steps.get(&step).map(|row| row.status) == Some(StepStatus::Running) {
        if let Err(error) = executor.reconcile(key) {
            return Failed(format!("reconcile of `{key}` failed: {error}"));
        }
        let Ok((_, after)) = executor.load(key) else {
            return Failed(format!("`{key}` disappeared"));
        };
        let status = after.steps.get(&step).map(|row| row.status);
        if status == Some(StepStatus::Running) {
            return Failed(format!("`{step}` is still running after a reconcile"));
        }
        if after.current.as_deref() != Some(step.as_str())
            || !matches!(status, Some(StepStatus::Failed | StepStatus::Escalated))
        {
            return Done(format!("reconciled `{key}`: `{step}` is {status:?} now"));
        }
    }
    match target.engine.retry(key, &step, DOCTOR_ACTOR) {
        Ok(_) => Done(format!("retried `{step}`")),
        Err(error) => Failed(format!("retry of `{step}` refused: {error}")),
    }
}

/// Moves the ticket's state label, which puts it back in a lane; never while a flow owns it.
fn requeue(target: &ActionTarget<'_>, state: &str) -> DoctorActionResult {
    let ticket = target.incident.candidate.ticket.as_str();
    if target.engine.has_active_run(ticket)
        || target.engine.has_active_run(&target.incident.candidate.key)
    {
        return Refused(format!("`{ticket}` still has an active flow"));
    }
    label(target, ticket, state)
}

fn label(target: &ActionTarget<'_>, ticket: &str, state: &str) -> DoctorActionResult {
    let action = SubprocessAction::new([AIDP_TICKET, AIDP_LABEL, KEY_PLACEHOLDER, state]);
    let runner = target.engine.executor().ports().runner.clone();
    let context = ActionContext {
        key: ticket,
        workspace: None,
        runner: runner.as_ref(),
    };
    match action.run(&context, &BTreeMap::new()) {
        ActionOutcome::Ok(_) => Done(format!("`{ticket}` labelled {state}")),
        other => Failed(format!("label {state} on `{ticket}` failed: {other:?}")),
    }
}

/// Aborts a flow that already ended, so its ticket is no longer owned by it, then puts the
/// ticket back in the lane of that flow: the test lane for a project-checkout flow, else dev.
fn abort_dead_flow(target: &ActionTarget<'_>) -> DoctorActionResult {
    let key = target.incident.candidate.key.as_str();
    let Some(entry) = target.engine.executor().store().index_get(key) else {
        return Refused(format!("no flow run for `{key}`"));
    };
    if !entry.status.is_terminal() {
        return Refused(format!(
            "`{key}` is {}: only a terminal flow is aborted",
            entry.status.as_wire()
        ));
    }
    let test_lane = target.engine.executor().load(key).is_ok_and(|(_, state)| {
        state.def.workspace == Some(aidp_flows::models::enums::WorkspaceMode::Project)
    });
    if entry.status != aidp_flows::FlowStatus::Aborted {
        if let Err(error) = target.engine.abort(key, DOCTOR_ACTOR) {
            return Failed(format!("abort of `{key}` failed: {error}"));
        }
    }
    let lane = if test_lane {
        STATE_TO_TEST
    } else {
        STATE_READY
    };
    label(target, &target.incident.candidate.ticket, lane)
}

/// `gh pr update-branch` by merge, the GitHub default; a rebase or force is refused.
fn update_pr_branch(target: &ActionTarget<'_>) -> DoctorActionResult {
    if REBASE_PARAMS
        .iter()
        .any(|name| param(target, name).is_some_and(|value| value != &Value::Bool(false)))
    {
        return Refused("only a merge update is allowed: no rebase, no force".to_string());
    }
    let state = target
        .engine
        .executor()
        .load(&target.incident.candidate.key)
        .ok()
        .map(|(_, state)| state);
    let known = state.as_ref().and_then(facts::pull_request);
    let number =
        text_param(target, PARAM_PR).or_else(|| known.as_ref().map(|pr| pr.number.to_string()));
    let repo = text_param(target, PARAM_REPO).or_else(|| known.and_then(|pr| pr.repository));
    let (Some(number), Some(repo)) = (number, repo) else {
        return Refused("no pull request number and repository".to_string());
    };
    if number.parse::<u64>().is_err() {
        return Refused(format!("`{number}` is not a pull request number"));
    }
    let mut spec = ProcessSpec::new(GH_PROGRAM)
        .args(GH_UPDATE_BRANCH.iter().map(|part| part.to_string()))
        .args([number.clone(), GH_REPO_FLAG.to_string(), repo.clone()]);
    spec.piped_stdout = true;
    spec.piped_stderr = true;
    match run_command(target, &spec) {
        Ok(0) => Done(format!("{repo}#{number} updated by merge")),
        Ok(code) => Failed(format!("gh pr update-branch exited {code}")),
        Err(error) => Failed(format!("gh pr update-branch failed: {error}")),
    }
}

fn run_command(target: &ActionTarget<'_>, spec: &ProcessSpec) -> std::io::Result<i32> {
    let runner = target.engine.executor().ports().runner.clone();
    let mut child = runner.spawn(spec)?;
    let drains: Vec<_> = [child.take_stdout(), child.take_stderr()]
        .into_iter()
        .flatten()
        .map(|mut stream| {
            std::thread::spawn(move || {
                let mut sink = Vec::new();
                let _ = stream.read_to_end(&mut sink);
            })
        })
        .collect();
    let deadline = Instant::now() + COMMAND_TIMEOUT;
    let code = loop {
        if let Some(code) = child.try_wait()? {
            break code;
        }
        if Instant::now() >= deadline {
            child.kill()?;
            let _ = child.wait();
            break -1;
        }
        std::thread::sleep(COMMAND_POLL);
    };
    for drain in drains {
        let _ = drain.join();
    }
    Ok(code)
}

/// Frees the Unity editor lock, only when its holder's flow is no longer active.
fn release_unity_lock(target: &ActionTarget<'_>) -> DoctorActionResult {
    let executor = target.engine.executor();
    let resource = text_param(target, PARAM_RESOURCE).or_else(|| {
        let project = match &target.incident.candidate.project_id {
            Some(id) => id.clone(),
            None => {
                let (_, state) = executor.load(&target.incident.candidate.key).ok()?;
                executor.ports().sessions.project_for(&state.ticket)?.id
            }
        };
        Some(resource_key(UNITY_EDITOR, &project))
    });
    let Some(resource) = resource else {
        return Refused("no Unity lock to name".to_string());
    };
    let (holder, _) = executor.resources().snapshot(&resource);
    let Some(holder) = holder else {
        return Refused(format!("nobody holds {resource}"));
    };
    if let Some(entry) = executor.store().index_get(&holder.flow_key) {
        if !entry.status.is_terminal() {
            return Refused(format!(
                "the holder `{}` is {}, not terminal",
                holder.flow_key,
                entry.status.as_wire()
            ));
        }
    }
    match executor.release_resource(&resource) {
        Some(released) => Done(format!("{resource} released from `{}`", released.flow_key)),
        None => Done(format!("{resource} was already free")),
    }
}
