//! The doctor's actions on the steps of a flow and on the runs behind them. Each re-checks its
//! precondition on the live state right before it acts; a failed precondition is a `refused` result.

use crate::doctor::actions::{
    now_utc, project_id_of, retry_step, text_param, unity_status_of, ActionTarget,
};
use crate::doctor::preconditions;
use crate::flows::executor::step_at_cursor;
use crate::flows::facts;
use crate::flows::registry::SessionState;
use crate::models::constants::doctor::{
    BUDGET_RAISED_NOTE, DOCTOR_ACTOR, MAX_TRANSIENT_RETRIES, PARAM_RECHECK, PARAM_STAGE,
    PARAM_STEP, RECHECK_CLEAN, UNITY_WAIT_MULTIPLIER,
};
use crate::models::constants::flow_facts::LIVE_STATUS_TIMEOUT;
use crate::models::enums::native_run_status::NativeRunStatus;
use crate::models::enums::{DoctorAction, DoctorActionResult, DoctorCategory, DoctorTrigger};
use aidp_flows::{FlowState, StepStatus};

use DoctorActionResult::{Done, Failed, Refused};

/// The flow state and the step the action is about: the step named, else the incident's.
fn flow_step(target: &ActionTarget<'_>) -> Result<(FlowState, String), String> {
    let key = target.incident.candidate.key.as_str();
    let step = text_param(target, PARAM_STEP)
        .or_else(|| target.incident.candidate.step.clone())
        .ok_or_else(|| "the incident names no step".to_string())?;
    let (_, state) = target
        .engine
        .executor()
        .load(key)
        .map_err(|_| format!("no flow run for `{key}`"))?;
    Ok((state, step))
}

fn status_of(target: &ActionTarget<'_>, step: &str) -> Option<StepStatus> {
    let (_, state) = target
        .engine
        .executor()
        .load(&target.incident.candidate.key)
        .ok()?;
    state.steps.get(step).map(|row| row.status)
}

fn journal(target: &ActionTarget<'_>, line: String) {
    target.engine.executor().journal(
        &target.incident.candidate.key,
        Some(DOCTOR_ACTOR),
        format!("doctor: {line}"),
    );
}

/// Closes the modal window of the project's Unity editor, then resumes the step.
pub(super) fn dismiss_unity_modal(target: &ActionTarget<'_>) -> DoctorActionResult {
    let Some(project) = project_id_of(target) else {
        return Refused("no project for the incident".to_string());
    };
    let Some(status) = unity_status_of(target) else {
        return Refused("the Unity status of the project cannot be read".to_string());
    };
    if let Err(reason) = preconditions::editor_blocked(&status, now_utc(target)) {
        return Refused(reason);
    }
    let closed = match target.unity_modals.dismiss(&project) {
        Ok(note) => note,
        Err(reason) => return Refused(reason),
    };
    match retry_step(target) {
        Done(detail) => Done(format!("{closed}; {detail}")),
        other => other,
    }
}

/// Resumes a Unity compile gate step with a longer wait, for this flow only.
pub(super) fn extend_unity_wait(target: &ActionTarget<'_>) -> DoctorActionResult {
    let (state, step) = match flow_step(target) {
        Ok(found) => found,
        Err(reason) => return Refused(reason),
    };
    if let Err(reason) = preconditions::extend_wait(&state, &step) {
        return Refused(reason);
    }
    let key = target.incident.candidate.key.as_str();
    target
        .engine
        .executor()
        .registry()
        .wait_extensions()
        .extend(key, UNITY_WAIT_MULTIPLIER);
    journal(
        target,
        format!("the compile gate of `{step}` waits {UNITY_WAIT_MULTIPLIER} times longer"),
    );
    match retry_step(target) {
        Done(detail) => Done(format!("wait x{UNITY_WAIT_MULTIPLIER}; {detail}")),
        other => other,
    }
}

/// Counts a failed report or verdict step as passed.
pub(super) fn mark_step_passed(target: &ActionTarget<'_>) -> DoctorActionResult {
    let (state, step) = match flow_step(target) {
        Ok(found) => found,
        Err(reason) => return Refused(reason),
    };
    let ticket = target.incident.candidate.ticket.as_str();
    let source = target.engine.executor().registry().pr_status_source();
    let has_pull_request = facts::pull_request(&state).is_some()
        || facts::status_within(source, ticket, LIVE_STATUS_TIMEOUT)
            .is_some_and(|status| status.get("number").is_some());
    let recheck_clean =
        text_param(target, PARAM_RECHECK).is_some_and(|value| value == RECHECK_CLEAN);
    if let Err(reason) = preconditions::mark_passed(&state, &step, has_pull_request, recheck_clean)
    {
        return Refused(reason);
    }
    match target
        .engine
        .mark_passed(&target.incident.candidate.key, &step, DOCTOR_ACTOR)
    {
        Ok(_) => Done(format!("`{step}` marked passed")),
        Err(error) => Failed(format!("mark passed of `{step}` refused: {error}")),
    }
}

/// Reads again the state of a step whose run ended, without resuming the run.
pub(super) fn reconcile_step(target: &ActionTarget<'_>) -> DoctorActionResult {
    let (state, step) = match flow_step(target) {
        Ok(found) => found,
        Err(reason) => return Refused(reason),
    };
    if let Err(reason) = preconditions::reconcile(&state, &step) {
        return Refused(reason);
    }
    let key = target.incident.candidate.key.as_str();
    if let Err(error) = target.engine.executor().reconcile(key) {
        return Failed(format!("reconcile of `{key}` failed: {error}"));
    }
    match status_of(target, &step) {
        Some(StepStatus::Running) => Failed(format!("`{step}` is still running after a reconcile")),
        Some(status) => Done(format!("reconciled `{key}`: `{step}` is {status:?} now")),
        None => Failed(format!("`{key}` disappeared")),
    }
}

/// Evaluates the budget guard again for a step that waits for budget, and lets it start when the
/// guard allows. A ticket ceiling is lifted only within `doctor.max_budget_raise` per 24 hours.
pub(super) fn nudge_budget(target: &ActionTarget<'_>) -> DoctorActionResult {
    let (state, step) = match flow_step(target) {
        Ok(found) => found,
        Err(reason) => return Refused(reason),
    };
    let needs_raise = match preconditions::nudge_budget(&state, &step) {
        Ok(needs_raise) => needs_raise,
        Err(reason) => return Refused(reason),
    };
    let key = target.incident.candidate.key.as_str();
    let ticket = target.incident.candidate.ticket.as_str();
    if needs_raise {
        let allowed = u64::from(target.settings.max_budget_raise);
        if allowed == 0 {
            return Refused(
                "the ticket ceiling needs a human: `doctor.max_budget_raise` is 0".to_string(),
            );
        }
        if target.budget_raises as u64 >= allowed {
            return Refused(format!(
                "`{ticket}` already got its {allowed} ceiling raises in 24 hours"
            ));
        }
        return match target.engine.approve(key, &step, DOCTOR_ACTOR) {
            Ok(_) => {
                journal(
                    target,
                    format!("ticket ceiling of `{ticket}` lifted for `{step}`"),
                );
                Done(format!(
                    "{BUDGET_RAISED_NOTE} for `{ticket}`; `{step}` goes on"
                ))
            }
            Err(error) => Failed(format!("the ceiling decision on `{step}` failed: {error}")),
        };
    }
    let template = step_at_cursor(&state)
        .and_then(|definition| definition.use_)
        .unwrap_or_default();
    let executor = target.engine.executor();
    let allows = executor.ports().budget.allows(&template);
    journal(
        target,
        format!(
            "budget re-evaluated for `{step}` ({template}): {}",
            if allows { "allowed" } else { "still refused" }
        ),
    );
    if !allows {
        return Failed(format!("the budget guard still refuses `{template}`"));
    }
    if let Err(error) = executor.reconcile(key) {
        return Failed(format!("reconcile of `{key}` failed: {error}"));
    }
    Done(format!("the budget allows `{template}`; `{step}` starts"))
}

/// Skips a failing step the template marks `optional`.
pub(super) fn skip_optional_step(target: &ActionTarget<'_>) -> DoctorActionResult {
    let (state, step) = match flow_step(target) {
        Ok(found) => found,
        Err(reason) => return Refused(reason),
    };
    if let Err(reason) = preconditions::skip_optional(&state, &step) {
        return Refused(reason);
    }
    match target
        .engine
        .skip(&target.incident.candidate.key, &step, DOCTOR_ACTOR)
    {
        Ok(_) => Done(format!("optional step `{step}` skipped")),
        Err(error) => Failed(format!("skip of `{step}` refused: {error}")),
    }
}

/// Reads the pull request head again and runs the `gh.merge` step again; never forces a merge.
pub(super) fn retry_merge_step(target: &ActionTarget<'_>) -> DoctorActionResult {
    let (state, step) = match flow_step(target) {
        Ok(found) => found,
        Err(reason) => return Refused(reason),
    };
    let ticket = target.incident.candidate.ticket.as_str();
    let source = target.engine.executor().registry().pr_status_source();
    let status = facts::refresh_within(source, ticket, LIVE_STATUS_TIMEOUT);
    let mergeable = preconditions::pr_mergeable(status.as_ref());
    if let Err(reason) = preconditions::retry_merge(&state, &step, mergeable) {
        return Refused(reason);
    }
    match target
        .engine
        .retry(&target.incident.candidate.key, &step, DOCTOR_ACTOR)
    {
        Ok(_) => Done(format!("`{step}` runs again on the head just read")),
        Err(error) => Failed(format!("retry of `{step}` refused: {error}")),
    }
}

/// Cancels the orphan session of a stuck step, then reconciles the step.
pub(super) fn kill_stale_session(target: &ActionTarget<'_>) -> DoctorActionResult {
    let candidate = &target.incident.candidate;
    if candidate.trigger != DoctorTrigger::StepStuck {
        return Refused("a session is killed for a stuck step only".to_string());
    }
    let (state, step) = match flow_step(target) {
        Ok(found) => found,
        Err(reason) => return Refused(reason),
    };
    if let Err(reason) = preconditions::reconcile(&state, &step) {
        return Refused(reason);
    }
    let mut killed = 0;
    if let Some(run_id) = candidate.run_id.as_deref() {
        killed += target.runs.cancel_sessions(run_id).unwrap_or(0);
        let sessions = &target.engine.executor().ports().sessions;
        if matches!(sessions.state(run_id), SessionState::Running) {
            sessions.cancel(run_id);
            killed += 1;
        }
    }
    let key = candidate.key.as_str();
    if let Err(error) = target.engine.executor().reconcile(key) {
        return Failed(format!("reconcile of `{key}` failed: {error}"));
    }
    match (killed, status_of(target, &step)) {
        (_, Some(StepStatus::Running)) if killed == 0 => Failed(format!(
            "no live session to kill and `{step}` is still running"
        )),
        (_, Some(StepStatus::Running)) => Done(format!(
            "{killed} session(s) cancelled; `{step}` reads them at the next reconcile"
        )),
        (_, status) => Done(format!(
            "{killed} session(s) cancelled; `{step}` is {status:?} now"
        )),
    }
}

/// Resumes a paused run from a named stage; that stage and the later ones run again.
pub(super) fn retry_from_stage(target: &ActionTarget<'_>) -> DoctorActionResult {
    let Some(stage) = text_param(target, PARAM_STAGE) else {
        return Refused("no stage named in `params.stage`".to_string());
    };
    let Some(run_id) = target.incident.candidate.run_id.as_deref() else {
        return Refused("the incident has no run".to_string());
    };
    let Some(view) = target.runs.get(run_id) else {
        return Refused(format!("run `{run_id}` is gone"));
    };
    if view.status != NativeRunStatus::Paused {
        return Refused(format!("run `{run_id}` is not paused"));
    }
    if !view.plan.iter().flatten().any(|name| *name == stage) {
        return Refused(format!("stage `{stage}` is not in run `{run_id}`"));
    }
    match target.runs.resume_from_stage(run_id, &stage) {
        Ok(()) => Done(format!("run `{run_id}` resumed from `{stage}`")),
        Err(error) => Failed(format!(
            "resume of run `{run_id}` from `{stage}` failed: {error}"
        )),
    }
}

/// Resumes with the fallback model once the provider stayed overloaded through every resumption.
pub(super) fn retry_with_fallback(target: &ActionTarget<'_>) -> DoctorActionResult {
    if target.diagnosis.category != DoctorCategory::ApiOverloaded {
        return Refused("the fallback model is for an overloaded provider only".to_string());
    }
    let model = match preconditions::fallback_model(target.settings.fallback_model.as_deref()) {
        Ok(model) => model,
        Err(reason) => return Refused(reason),
    };
    let retries = target
        .incident
        .actions
        .iter()
        .filter(|done| done.action == DoctorAction::RetryStep)
        .count();
    if retries < MAX_TRANSIENT_RETRIES {
        return Refused(format!(
            "{retries} resumptions so far: the fallback comes after {MAX_TRANSIENT_RETRIES}"
        ));
    }
    let Some(run_id) = target.incident.candidate.run_id.as_deref() else {
        return Refused("the incident has no run".to_string());
    };
    if let Err(error) = target.runs.set_model(run_id, &model) {
        return Failed(format!("model of run `{run_id}` not set: {error}"));
    }
    match retry_step(target) {
        Done(detail) => Done(format!("model {model}; {detail}")),
        other => other,
    }
}
