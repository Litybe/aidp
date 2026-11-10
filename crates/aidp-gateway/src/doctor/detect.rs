//! Detection of broken flows and runs. Pure: it reads snapshots and takes no lock, so it
//! never runs inside the reconcile path and never blocks it.

use crate::doctor::transient::{classify, is_art_template};
use crate::flows::executor::step_at_cursor;
use crate::flows::pipeline_inputs::ticket_key;
use crate::models::constants::doctor::{
    BUG_CAPPED_STEP, CAP_INCIDENT_SEPARATOR, DOCTOR_TEMPLATE, EFFECT_SEPARATOR, FRESH_RUN_HOURS,
    INCIDENT_SEPARATOR, RUN_CAUSE_PREFIX, RUN_ENDED_GRACE_MINUTES, START_PIPELINE_SUFFIX,
    WAITING_EXTERNAL_MINUTES,
};
use crate::models::enums::native_run_status::NativeRunStatus;
use crate::models::enums::{DoctorLiveRun, DoctorTrigger};
use crate::models::native_run_view::NativeRunView;
use crate::models::{DoctorCandidate, DoctorFlowSnapshot};
use aidp_flows::{effect_id, FlowStatus, StepKind, StepReason, StepState, StepStatus};
use std::collections::BTreeSet;

const MILLIS_PER_MINUTE: i64 = 60_000;
const MILLIS_PER_HOUR: i64 = 60 * MILLIS_PER_MINUTE;

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
    /// Epoch milliseconds the condition of incident `id` was first seen: the doctor remembers
    /// it between passes, so a transient state is never an incident at its first sight.
    pub first_seen: &'a dyn Fn(&str) -> i64,
    /// Text of a run's pause reason and stage replies, read to tell a passing fault from the
    /// art pipeline's own findings.
    pub run_text: &'a dyn Fn(&NativeRunView) -> String,
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

/// Every broken flow and run that is still live, one candidate per ticket and cause. Flows
/// that are going come first, the most recent first; a run that was replaced, a flow that
/// ended and a run too old are not incidents.
pub fn detect(input: &DetectInput<'_>) -> Vec<DoctorCandidate> {
    let mut found: Vec<DoctorCandidate> = input
        .flows
        .iter()
        .filter_map(|snapshot| flow_candidate(input, snapshot))
        .collect();
    let mut seen: BTreeSet<String> = found.iter().map(|candidate| candidate.id.clone()).collect();
    for run in input.runs.iter().filter(|run| is_broken_run(run)) {
        let Some(candidate) = run_candidate(input, run) else {
            continue;
        };
        if seen.insert(candidate.id.clone()) {
            found.push(candidate);
        }
    }
    found.sort_by(|a, b| {
        b.live
            .cmp(&a.live)
            .then_with(|| b.activity_ms.cmp(&a.activity_ms))
    });
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
    let detail = format!(
        "{step} {} ({}): {}",
        state.status.as_wire(),
        row.reason.as_deref().unwrap_or("-"),
        row.detail.as_deref().unwrap_or("-")
    );
    if step == BUG_CAPPED_STEP {
        return Some(bug_cap_candidate(snapshot, step, row, detail));
    }
    if chosen_by_human_or_product(row) || art_findings(input, snapshot, step, row) {
        return None;
    }
    Some(candidate(snapshot, trigger, step, row, detail))
}

/// The incident of a ticket whose test still fails after its bugs reached the cap. Its id carries
/// the start time of the flow, so a later cap of the same ticket is a new incident.
fn bug_cap_candidate(
    snapshot: &DoctorFlowSnapshot,
    step: &str,
    row: &StepState,
    detail: String,
) -> DoctorCandidate {
    let mut found = candidate(
        snapshot,
        DoctorTrigger::BugsNotConverging,
        step,
        row,
        detail,
    );
    let started = snapshot
        .state
        .started_at
        .as_deref()
        .and_then(millis_of)
        .unwrap_or(0);
    found.id = format!("{}{CAP_INCIDENT_SEPARATOR}{started}", found.id);
    found
}

/// True when the failing step ran an art pipeline that stopped on art findings: the art
/// pipeline's own work, not an incident. A passing fault of an art run still is one.
fn art_findings(
    input: &DetectInput<'_>,
    snapshot: &DoctorFlowSnapshot,
    step: &str,
    row: &StepState,
) -> bool {
    let effect = effect_id(step, row.iteration, START_PIPELINE_SUFFIX);
    let run = snapshot
        .sessions
        .get(&effect)
        .and_then(|id| input.runs.iter().find(|run| run.run_id == *id));
    let template = run.map(|run| run.template_id.clone()).or_else(|| {
        snapshot
            .state
            .def
            .steps
            .iter()
            .find(|definition| definition.id == step)
            .and_then(|definition| definition.use_.clone())
    });
    if !template.as_deref().is_some_and(is_art_template) {
        return false;
    }
    let mut text = format!(
        "{} {}",
        row.reason.as_deref().unwrap_or_default(),
        row.detail.as_deref().unwrap_or_default()
    );
    if let Some(run) = run {
        text.push(' ');
        text.push_str(&(input.run_text)(run));
    }
    classify(&text).is_none()
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

/// Milliseconds the doctor itself has seen step `step` stuck.
fn seen_for(
    input: &DetectInput<'_>,
    snapshot: &DoctorFlowSnapshot,
    step: &str,
    row: &StepState,
) -> i64 {
    let id = step_incident_id(&snapshot.state.key, step, row.iteration);
    input.now_ms - (input.first_seen)(&id)
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
            let grace = RUN_ENDED_GRACE_MINUTES * MILLIS_PER_MINUTE;
            let (waited, limit, seen) = match session.map(|id| (input.live)(id)) {
                Some(DoctorLiveRun::Running) => return None,
                // The run ended: the engine reconciles within a few ticks. Not before the grace
                // since the end, nor before the doctor itself saw it for that long.
                Some(DoctorLiveRun::Ended(at)) => (
                    elapsed(at.as_deref().or(row.started_at.as_deref()))
                        .min(seen_for(input, snapshot, step, row)),
                    grace,
                    "ended",
                ),
                Some(DoctorLiveRun::Gone) => (
                    elapsed(row.started_at.as_deref()).min(seen_for(input, snapshot, step, row)),
                    stuck_ms,
                    "gone",
                ),
                None => (
                    elapsed(row.started_at.as_deref()).min(seen_for(input, snapshot, step, row)),
                    stuck_ms,
                    "never started",
                ),
            };
            (waited > limit).then(|| {
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
        live: is_going(state.status),
        activity_ms: [
            row.finished_at.as_deref(),
            row.started_at.as_deref(),
            state.started_at.as_deref(),
        ]
        .into_iter()
        .flatten()
        .find_map(millis_of)
        .unwrap_or(0),
    }
}

fn is_going(status: FlowStatus) -> bool {
    matches!(
        status,
        FlowStatus::Running | FlowStatus::WaitingExternal | FlowStatus::WaitingHuman
    )
}

/// A paused run that is still live, filed under the flow step that launched it when there is
/// one. Live means: the run of the current step of a flow that is not over, or, outside any
/// flow, a run of the last 24 hours that no newer run of the same ticket and template replaced.
/// A run of the art pipeline is an incident only when a passing fault stopped it.
fn run_candidate(input: &DetectInput<'_>, run: &NativeRunView) -> Option<DoctorCandidate> {
    if is_art_template(&run.template_id) && classify(&(input.run_text)(run)).is_none() {
        return None;
    }
    let detail = format!(
        "run {} ({}) paused: {}",
        run.run_id,
        run.template_id,
        run.reason.as_deref().unwrap_or("-")
    );
    let at = run
        .ended_at
        .as_deref()
        .and_then(millis_of)
        .or_else(|| millis_of(&run.started_at))
        .unwrap_or(0);
    let mut owned = false;
    for snapshot in input.flows {
        let owner = snapshot
            .sessions
            .iter()
            .find(|(_, id)| **id == run.run_id)
            .and_then(|(effect, _)| step_of_effect(effect));
        let Some((step, iteration)) = owner else {
            continue;
        };
        owned = true;
        let state = &snapshot.state;
        let is_current = state.current.as_deref() == Some(step.as_str())
            && state.steps.get(&step).map(|row| row.iteration) == Some(iteration);
        if !is_going(state.status) || !is_current {
            continue;
        }
        return Some(DoctorCandidate {
            id: step_incident_id(&state.key, &step, iteration),
            key: state.key.clone(),
            ticket: ticket_key(state).to_string(),
            trigger: DoctorTrigger::RunPaused,
            step: Some(step),
            iteration: Some(iteration),
            run_id: Some(run.run_id.clone()),
            project_id: None,
            detail,
            live: true,
            activity_ms: at,
        });
    }
    if owned || input.now_ms - at > FRESH_RUN_HOURS * MILLIS_PER_HOUR || replaced(input, run) {
        return None;
    }
    let key = run
        .request
        .ticket
        .clone()
        .or_else(|| run.request.target.clone())
        .unwrap_or_else(|| run.run_id.clone());
    Some(DoctorCandidate {
        id: run_incident_id(&key, &run.run_id),
        ticket: key.clone(),
        key,
        trigger: DoctorTrigger::RunPaused,
        step: None,
        iteration: None,
        run_id: Some(run.run_id.clone()),
        project_id: Some(run.request.project_id.clone()),
        detail,
        live: false,
        activity_ms: at,
    })
}

/// True when a newer run of the same ticket (or target) and template exists.
fn replaced(input: &DetectInput<'_>, run: &NativeRunView) -> bool {
    let subject = |other: &NativeRunView| {
        other
            .request
            .ticket
            .clone()
            .or_else(|| other.request.target.clone())
    };
    let started = millis_of(&run.started_at).unwrap_or(0);
    input.runs.iter().any(|other| {
        other.run_id != run.run_id
            && other.template_id == run.template_id
            && subject(other) == subject(run)
            && millis_of(&other.started_at).unwrap_or(0) > started
    })
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
