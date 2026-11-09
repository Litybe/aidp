//! Detection of broken flows and runs. Pure: it reads snapshots and takes no lock, so it
//! never runs inside the reconcile path and never blocks it.

use crate::flows::executor::step_at_cursor;
use crate::flows::pipeline_inputs::ticket_key;
use crate::models::constants::doctor::{
    DOCTOR_TEMPLATE, EFFECT_SEPARATOR, INCIDENT_SEPARATOR, RUN_CAUSE_PREFIX, START_PIPELINE_SUFFIX,
    WAITING_EXTERNAL_MINUTES,
};
use crate::models::enums::native_run_status::NativeRunStatus;
use crate::models::enums::{DoctorLiveRun, DoctorTrigger};
use crate::models::native_run_view::NativeRunView;
use crate::models::{DoctorCandidate, DoctorFlowSnapshot};
use aidp_flows::{effect_id, FlowStatus, StepKind, StepReason, StepState, StepStatus};
use std::collections::BTreeSet;

const MILLIS_PER_MINUTE: i64 = 60_000;

/// Escalation reasons a human or the product chose: not a pipeline failure.
const HUMAN_OR_PRODUCT_REASONS: [StepReason; 5] = [
    StepReason::MaxIterations,
    StepReason::Rejected,
    StepReason::TicketBudget,
    StepReason::Aborted,
    StepReason::ManualSkip,
];

/// What detection reads.
pub struct DetectInput<'a> {
    pub flows: &'a [DoctorFlowSnapshot],
    pub runs: &'a [NativeRunView],
    /// Live state of a native run or session id.
    pub live: &'a dyn Fn(&str) -> DoctorLiveRun,
    pub now_ms: i64,
    pub stuck_minutes: u32,
}

/// Id of the incident of step `step` at `iteration` of run `key`.
pub fn step_incident_id(key: &str, step: &str, iteration: u32) -> String {
    format!("{key}{INCIDENT_SEPARATOR}{step}@{iteration}")
}

/// Id of the incident of native run `run_id` outside any flow, filed under `key`.
pub fn run_incident_id(key: &str, run_id: &str) -> String {
    format!("{key}{INCIDENT_SEPARATOR}{RUN_CAUSE_PREFIX}{run_id}")
}

/// Epoch milliseconds of an RFC 3339 time.
pub fn millis_of(at: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(at)
        .ok()
        .map(|time| time.timestamp_millis())
}

/// Every broken flow and run, one candidate per ticket and cause, flow triggers first.
pub fn detect(input: &DetectInput<'_>) -> Vec<DoctorCandidate> {
    let mut found: Vec<DoctorCandidate> = input
        .flows
        .iter()
        .filter_map(|snapshot| flow_candidate(input, snapshot))
        .collect();
    let mut seen: BTreeSet<String> = found.iter().map(|candidate| candidate.id.clone()).collect();
    for run in input.runs.iter().filter(|run| is_broken_run(run)) {
        let candidate = run_candidate(input.flows, run);
        if seen.insert(candidate.id.clone()) {
            found.push(candidate);
        }
    }
    found
}

/// A paused native run that is not an investigation: a failed doctor never calls a doctor.
fn is_broken_run(run: &NativeRunView) -> bool {
    run.status == NativeRunStatus::Paused && run.template_id != DOCTOR_TEMPLATE
}

fn flow_candidate(
    input: &DetectInput<'_>,
    snapshot: &DoctorFlowSnapshot,
) -> Option<DoctorCandidate> {
    let state = &snapshot.state;
    let trigger = match state.status {
        FlowStatus::Failed => DoctorTrigger::FlowFailed,
        FlowStatus::Escalated => DoctorTrigger::FlowEscalated,
        FlowStatus::Running | FlowStatus::WaitingExternal | FlowStatus::WaitingHuman => {
            return live_step_candidate(input, snapshot);
        }
        FlowStatus::Done | FlowStatus::Aborted | FlowStatus::Paused | FlowStatus::Pausing => {
            return None;
        }
    };
    let (step, row) = failing_step(state)?;
    if chosen_by_human_or_product(row) {
        return None;
    }
    Some(candidate(
        snapshot,
        trigger,
        step,
        row,
        format!(
            "{step} {} ({}): {}",
            state.status.as_wire(),
            row.reason.as_deref().unwrap_or("-"),
            row.detail.as_deref().unwrap_or("-")
        ),
    ))
}

/// The current step when it failed or escalated, else the last one that did.
fn failing_step(state: &aidp_flows::FlowState) -> Option<(&str, &StepState)> {
    let failed = |row: &StepState| matches!(row.status, StepStatus::Failed | StepStatus::Escalated);
    if let Some(current) = state.current.as_deref() {
        if let Some(row) = state.steps.get(current).filter(|row| failed(row)) {
            return Some((current, row));
        }
    }
    state
        .steps
        .iter()
        .filter(|(_, row)| failed(row))
        .max_by(|a, b| a.1.finished_at.cmp(&b.1.finished_at))
        .map(|(id, row)| (id.as_str(), row))
}

fn chosen_by_human_or_product(row: &StepState) -> bool {
    row.status == StepStatus::Escalated
        && HUMAN_OR_PRODUCT_REASONS
            .iter()
            .any(|reason| row.reason.as_deref() == Some(reason.as_str()))
}

/// A running step whose run is over or gone, or a step waiting for something external too long.
fn live_step_candidate(
    input: &DetectInput<'_>,
    snapshot: &DoctorFlowSnapshot,
) -> Option<DoctorCandidate> {
    let state = &snapshot.state;
    let step = state.current.as_deref()?;
    let row = state.steps.get(step)?;
    let kind = step_at_cursor(state)?.kind;
    let stuck_ms = i64::from(input.stuck_minutes) * MILLIS_PER_MINUTE;
    let elapsed = |since: Option<&str>| {
        since
            .and_then(millis_of)
            .map(|at| input.now_ms - at)
            .unwrap_or(0)
    };
    match (row.status, kind) {
        (StepStatus::Running, StepKind::Pipeline) => {
            let effect = effect_id(step, row.iteration, START_PIPELINE_SUFFIX);
            let session = snapshot.sessions.get(&effect);
            let (since, seen) = match session.map(|id| (input.live)(id)) {
                Some(DoctorLiveRun::Running) => return None,
                Some(DoctorLiveRun::Ended(at)) => (at.or_else(|| row.started_at.clone()), "ended"),
                Some(DoctorLiveRun::Gone) => (row.started_at.clone(), "gone"),
                None => (row.started_at.clone(), "never started"),
            };
            (elapsed(since.as_deref()) > stuck_ms).then(|| {
                candidate(
                    snapshot,
                    DoctorTrigger::StepStuck,
                    step,
                    row,
                    format!("{step} running while its run {seen}"),
                )
            })
        }
        (StepStatus::Running, StepKind::Action) if !snapshot.external => {
            let idle = !snapshot.action_inflight;
            (idle && elapsed(row.started_at.as_deref()) > stuck_ms).then(|| {
                candidate(
                    snapshot,
                    DoctorTrigger::StepStuck,
                    step,
                    row,
                    format!("{step} running with no action process"),
                )
            })
        }
        (StepStatus::WaitingExternal, kind) if !(snapshot.external && kind == StepKind::Action) => {
            let limit = WAITING_EXTERNAL_MINUTES * MILLIS_PER_MINUTE;
            (elapsed(row.started_at.as_deref()) > limit).then(|| {
                candidate(
                    snapshot,
                    DoctorTrigger::WaitingTooLong,
                    step,
                    row,
                    format!(
                        "{step} waiting_external ({}) for over {WAITING_EXTERNAL_MINUTES} min",
                        row.reason.as_deref().unwrap_or("-")
                    ),
                )
            })
        }
        _ => None,
    }
}

fn candidate(
    snapshot: &DoctorFlowSnapshot,
    trigger: DoctorTrigger,
    step: &str,
    row: &StepState,
    detail: String,
) -> DoctorCandidate {
    let state = &snapshot.state;
    let effect = effect_id(step, row.iteration, START_PIPELINE_SUFFIX);
    DoctorCandidate {
        id: step_incident_id(&state.key, step, row.iteration),
        key: state.key.clone(),
        ticket: ticket_key(state).to_string(),
        trigger,
        step: Some(step.to_string()),
        iteration: Some(row.iteration),
        run_id: snapshot.sessions.get(&effect).cloned(),
        project_id: None,
        detail,
    }
}

/// A paused run, filed under the flow step that launched it when there is one.
fn run_candidate(flows: &[DoctorFlowSnapshot], run: &NativeRunView) -> DoctorCandidate {
    let detail = format!(
        "run {} ({}) paused: {}",
        run.run_id,
        run.template_id,
        run.reason.as_deref().unwrap_or("-")
    );
    for snapshot in flows {
        let owner = snapshot
            .sessions
            .iter()
            .find(|(_, id)| **id == run.run_id)
            .and_then(|(effect, _)| step_of_effect(effect));
        if let Some((step, iteration)) = owner {
            let state = &snapshot.state;
            return DoctorCandidate {
                id: step_incident_id(&state.key, &step, iteration),
                key: state.key.clone(),
                ticket: ticket_key(state).to_string(),
                trigger: DoctorTrigger::RunPaused,
                step: Some(step),
                iteration: Some(iteration),
                run_id: Some(run.run_id.clone()),
                project_id: None,
                detail,
            };
        }
    }
    let key = run
        .request
        .ticket
        .clone()
        .or_else(|| run.request.target.clone())
        .unwrap_or_else(|| run.run_id.clone());
    DoctorCandidate {
        id: run_incident_id(&key, &run.run_id),
        ticket: key.clone(),
        key,
        trigger: DoctorTrigger::RunPaused,
        step: None,
        iteration: None,
        run_id: Some(run.run_id.clone()),
        project_id: Some(run.request.project_id.clone()),
        detail,
    }
}

/// `(step, iteration)` of a `<step>:<iteration>:start_pipeline` effect id.
pub fn step_of_effect(effect: &str) -> Option<(String, u32)> {
    let mut parts = effect.rsplitn(3, EFFECT_SEPARATOR);
    let kind = parts.next()?;
    let iteration = parts.next()?.parse().ok()?;
    let step = parts.next()?;
    (kind == START_PIPELINE_SUFFIX).then(|| (step.to_string(), iteration))
}

#[cfg(test)]
#[path = "../../tests/unit/doctor_detect_tests.rs"]
mod tests;
