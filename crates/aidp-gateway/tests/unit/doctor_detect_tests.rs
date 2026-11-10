use super::*;
use crate::models::constants::doctor::WAITING_EXTERNAL_MINUTES;
use crate::models::native_run_start::StartNativeRun;
use aidp_flows::{FlowDef, FlowState, Frame, FrameKind};
use std::collections::BTreeMap;
use std::path::PathBuf;

const FLOW: &str = include_str!("../fixtures/doctor/unit_flow.yaml");
const KEY: &str = "GH-7";
const TICKET: &str = "GH-7";
const REPO: &str = "pixelhood";
const RUN_ID: &str = "run-1";
const OTHER_RUN_ID: &str = "run-2";
const PREPARE: &str = "prepare";
const IMPLEMENT: &str = "implement";
const WAIT_CI: &str = "wait_ci";
const MERGE: &str = "merge";
const NOW: &str = "2026-11-09T12:00:00Z";
const LONG_AGO: &str = "2026-11-09T10:00:00Z";
const JUST_NOW: &str = "2026-11-09T11:58:00Z";
const STUCK_MINUTES: u32 = 10;
const PROJECT: &str = "p1";
const UNITY_TEMPLATE: &str = "unity-implement";
const ART_TEMPLATE: &str = "art-produce";
const ART_FINDINGS: &str = "correction did not converge";
const TWO_DAYS_AGO: &str = "2026-11-07T12:00:00Z";
const TICK_AGO_MS: i64 = 11 * 60_000;

fn now_ms() -> i64 {
    millis_of(NOW).unwrap()
}

fn index_of(def: &FlowDef, step: &str) -> usize {
    def.steps.iter().position(|row| row.id == step).unwrap()
}

/// A run of the unit flow positioned on `step` with `row` as its state.
fn flow(status: FlowStatus, step: &str, row: StepState) -> DoctorFlowSnapshot {
    let def = FlowDef::from_yaml(FLOW).unwrap();
    let mut state = FlowState::new(KEY, def.clone());
    state.ticket = serde_json::json!({ "key": TICKET });
    state.status = status;
    state.current = Some(step.to_string());
    state.started_at = Some(LONG_AGO.to_string());
    state.stack.push(Frame {
        kind: FrameKind::Root,
        prefix: String::new(),
        owner: None,
        steps: def.steps.clone(),
        index: index_of(&def, step),
    });
    state.steps.insert(step.to_string(), row);
    DoctorFlowSnapshot {
        repo: REPO.to_string(),
        state,
        sessions: BTreeMap::new(),
        external: false,
        action_inflight: false,
    }
}

fn row(status: StepStatus, reason: Option<StepReason>, started_at: &str) -> StepState {
    StepState {
        status,
        reason: reason.map(|reason| reason.as_str().to_string()),
        started_at: Some(started_at.to_string()),
        ..StepState::default()
    }
}

fn with_run(mut snapshot: DoctorFlowSnapshot, step: &str, run_id: &str) -> DoctorFlowSnapshot {
    let iteration = snapshot.state.steps[step].iteration;
    snapshot.sessions.insert(
        effect_id(step, iteration, START_PIPELINE_SUFFIX),
        run_id.to_string(),
    );
    snapshot
}

fn run(run_id: &str, template: &str, status: NativeRunStatus) -> NativeRunView {
    NativeRunView {
        run_id: run_id.to_string(),
        template_id: template.to_string(),
        status,
        reason: Some("implement: needs_review".to_string()),
        workspace: PathBuf::new(),
        run_dir: PathBuf::new(),
        request: StartNativeRun {
            project_id: PROJECT.to_string(),
            template_id: template.to_string(),
            ticket: Some(TICKET.to_string()),
            ..StartNativeRun::default()
        },
        pending_gate: None,
        started_at: LONG_AGO.to_string(),
        ended_at: Some(LONG_AGO.to_string()),
        plan: Vec::new(),
        stages: vec![crate::models::native_stage_view::NativeStageView::pending(
            IMPLEMENT,
        )],
        autonomous: true,
        assumptions: Vec::new(),
        escalations: Vec::new(),
    }
}

fn detect_seen(
    flows: &[DoctorFlowSnapshot],
    runs: &[NativeRunView],
    live: DoctorLiveRun,
    seen_ago_ms: i64,
) -> Vec<DoctorCandidate> {
    let answer = move |_: &str| live.clone();
    let first_seen = move |_: &str| now_ms() - seen_ago_ms;
    let text = |view: &NativeRunView| view.reason.clone().unwrap_or_default();
    detect(&DetectInput {
        flows,
        runs,
        live: &answer,
        now_ms: now_ms(),
        stuck_minutes: STUCK_MINUTES,
        first_seen: &first_seen,
        run_text: &text,
    })
}

/// The doctor has watched the condition for ten minutes.
fn detect_with(
    flows: &[DoctorFlowSnapshot],
    runs: &[NativeRunView],
    live: DoctorLiveRun,
) -> Vec<DoctorCandidate> {
    detect_seen(flows, runs, live, TICK_AGO_MS)
}

#[test]
fn a_failed_flow_is_one_incident_on_its_failing_step() {
    let snapshot = flow(
        FlowStatus::Failed,
        IMPLEMENT,
        row(
            StepStatus::Failed,
            Some(StepReason::PipelineFailed),
            LONG_AGO,
        ),
    );
    let found = detect_with(&[snapshot], &[], DoctorLiveRun::Running);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].trigger, DoctorTrigger::FlowFailed);
    assert_eq!(found[0].id, step_incident_id(KEY, IMPLEMENT, 0));
    assert_eq!(found[0].ticket, TICKET);
}

#[test]
fn an_escalated_flow_by_a_business_error_is_an_incident() {
    let snapshot = flow(
        FlowStatus::Escalated,
        MERGE,
        row(
            StepStatus::Escalated,
            Some(StepReason::BusinessError),
            LONG_AGO,
        ),
    );
    let found = detect_with(&[snapshot], &[], DoctorLiveRun::Running);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].trigger, DoctorTrigger::FlowEscalated);
}

#[test]
fn a_human_or_product_escalation_is_out_of_scope() {
    for reason in [
        StepReason::MaxIterations,
        StepReason::Rejected,
        StepReason::TicketBudget,
    ] {
        let snapshot = flow(
            FlowStatus::Escalated,
            IMPLEMENT,
            row(StepStatus::Escalated, Some(reason), LONG_AGO),
        );
        assert!(
            detect_with(&[snapshot], &[], DoctorLiveRun::Running).is_empty(),
            "{reason:?} is not a pipeline failure"
        );
    }
}

#[test]
fn a_paused_run_of_a_flow_step_shares_the_flow_incident() {
    let snapshot = with_run(
        flow(
            FlowStatus::Failed,
            IMPLEMENT,
            row(
                StepStatus::Failed,
                Some(StepReason::PipelineFailed),
                LONG_AGO,
            ),
        ),
        IMPLEMENT,
        RUN_ID,
    );
    let paused = run(RUN_ID, UNITY_TEMPLATE, NativeRunStatus::Paused);
    let found = detect_with(&[snapshot], &[paused], DoctorLiveRun::Running);
    assert_eq!(found.len(), 1, "one incident per ticket and cause");
    assert_eq!(found[0].trigger, DoctorTrigger::FlowFailed);
    assert_eq!(found[0].run_id.as_deref(), Some(RUN_ID));
}

#[test]
fn a_paused_run_is_an_incident_on_its_step() {
    let snapshot = with_run(
        flow(
            FlowStatus::Running,
            IMPLEMENT,
            row(StepStatus::Running, None, JUST_NOW),
        ),
        IMPLEMENT,
        RUN_ID,
    );
    let paused = run(RUN_ID, UNITY_TEMPLATE, NativeRunStatus::Paused);
    let found = detect_with(&[snapshot], &[paused], DoctorLiveRun::Running);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].trigger, DoctorTrigger::RunPaused);
    assert_eq!(found[0].id, step_incident_id(KEY, IMPLEMENT, 0));
}

#[test]
fn a_paused_run_outside_any_flow_is_filed_under_its_ticket() {
    let paused = run(OTHER_RUN_ID, UNITY_TEMPLATE, NativeRunStatus::Paused);
    let found = detect_with(&[], &[paused], DoctorLiveRun::Running);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].id, run_incident_id(TICKET, OTHER_RUN_ID));
    assert_eq!(found[0].project_id.as_deref(), Some(PROJECT));
}

#[test]
fn a_paused_investigation_never_opens_an_incident() {
    let paused = run(OTHER_RUN_ID, DOCTOR_TEMPLATE, NativeRunStatus::Paused);
    assert!(detect_with(&[], &[paused], DoctorLiveRun::Running).is_empty());
}

#[test]
fn a_running_step_whose_run_ended_long_ago_is_stuck() {
    let snapshot = with_run(
        flow(
            FlowStatus::Running,
            IMPLEMENT,
            row(StepStatus::Running, None, LONG_AGO),
        ),
        IMPLEMENT,
        RUN_ID,
    );
    let ended = DoctorLiveRun::Ended(Some(LONG_AGO.to_string()));
    let found = detect_with(&[snapshot], &[], ended);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].trigger, DoctorTrigger::StepStuck);
}

#[test]
fn a_running_step_whose_run_just_ended_or_still_runs_is_not_stuck() {
    let snapshot = with_run(
        flow(
            FlowStatus::Running,
            IMPLEMENT,
            row(StepStatus::Running, None, LONG_AGO),
        ),
        IMPLEMENT,
        RUN_ID,
    );
    let recent = DoctorLiveRun::Ended(Some(JUST_NOW.to_string()));
    assert!(detect_with(std::slice::from_ref(&snapshot), &[], recent).is_empty());
    assert!(detect_with(&[snapshot], &[], DoctorLiveRun::Running).is_empty());
}

#[test]
fn a_running_step_with_no_live_session_is_stuck() {
    let gone = with_run(
        flow(
            FlowStatus::Running,
            IMPLEMENT,
            row(StepStatus::Running, None, LONG_AGO),
        ),
        IMPLEMENT,
        RUN_ID,
    );
    assert_eq!(detect_with(&[gone], &[], DoctorLiveRun::Gone).len(), 1);
    let never = flow(
        FlowStatus::Running,
        IMPLEMENT,
        row(StepStatus::Running, None, LONG_AGO),
    );
    assert_eq!(detect_with(&[never], &[], DoctorLiveRun::Running).len(), 1);
}

#[test]
fn a_running_action_is_stuck_only_without_a_worker() {
    let idle = flow(
        FlowStatus::Running,
        PREPARE,
        row(StepStatus::Running, None, LONG_AGO),
    );
    assert_eq!(
        detect_with(std::slice::from_ref(&idle), &[], DoctorLiveRun::Running).len(),
        1
    );
    let mut busy = idle;
    busy.action_inflight = true;
    assert!(detect_with(&[busy], &[], DoctorLiveRun::Running).is_empty());
}

#[test]
fn a_long_external_wait_is_an_incident_and_a_short_one_is_not() {
    let long = flow(
        FlowStatus::WaitingExternal,
        IMPLEMENT,
        row(
            StepStatus::WaitingExternal,
            Some(StepReason::Budget),
            LONG_AGO,
        ),
    );
    let found = detect_with(&[long], &[], DoctorLiveRun::Running);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].trigger, DoctorTrigger::WaitingTooLong);
    let short_start =
        chrono::DateTime::from_timestamp_millis(now_ms() - (WAITING_EXTERNAL_MINUTES - 1) * 60_000)
            .unwrap()
            .to_rfc3339();
    let short = flow(
        FlowStatus::WaitingExternal,
        WAIT_CI,
        row(StepStatus::WaitingExternal, None, &short_start),
    );
    assert!(detect_with(&[short], &[], DoctorLiveRun::Running).is_empty());
}

#[test]
fn an_external_executor_action_wait_belongs_to_its_host() {
    let mut snapshot = flow(
        FlowStatus::WaitingExternal,
        PREPARE,
        row(StepStatus::WaitingExternal, None, LONG_AGO),
    );
    snapshot.external = true;
    assert!(detect_with(&[snapshot], &[], DoctorLiveRun::Running).is_empty());
}

#[test]
fn a_paused_or_done_flow_is_left_alone() {
    for status in [FlowStatus::Paused, FlowStatus::Done, FlowStatus::Aborted] {
        let snapshot = flow(status, IMPLEMENT, row(StepStatus::Failed, None, LONG_AGO));
        assert!(detect_with(&[snapshot], &[], DoctorLiveRun::Gone).is_empty());
    }
}

#[test]
fn the_step_of_an_effect_id_is_read_back() {
    let effect = effect_id("publish@implement/git_push", 2, START_PIPELINE_SUFFIX);
    assert_eq!(
        step_of_effect(&effect),
        Some(("publish@implement/git_push".to_string(), 2))
    );
    assert_eq!(step_of_effect("implement:0:run_action"), None);
}

fn paused_flow_step(status: FlowStatus) -> DoctorFlowSnapshot {
    with_run(
        flow(status, IMPLEMENT, row(StepStatus::Running, None, JUST_NOW)),
        IMPLEMENT,
        RUN_ID,
    )
}

fn run_started(run_id: &str, template: &str, started: &str) -> NativeRunView {
    let mut view = run(run_id, template, NativeRunStatus::Paused);
    view.started_at = started.to_string();
    view.ended_at = Some(started.to_string());
    view
}

#[test]
fn a_paused_run_of_a_flow_that_is_over_is_not_an_incident() {
    for status in [FlowStatus::Done, FlowStatus::Aborted] {
        let snapshot = paused_flow_step(status);
        let paused = run(RUN_ID, UNITY_TEMPLATE, NativeRunStatus::Paused);
        assert!(
            detect_with(&[snapshot], &[paused], DoctorLiveRun::Running).is_empty(),
            "{status:?}"
        );
    }
}

#[test]
fn a_paused_run_that_is_no_longer_the_current_step_is_not_an_incident() {
    let mut snapshot = paused_flow_step(FlowStatus::Running);
    // The flow moved on: its current step is another one.
    snapshot.state.current = Some(MERGE.to_string());
    snapshot
        .state
        .steps
        .insert(MERGE.to_string(), row(StepStatus::Running, None, JUST_NOW));
    let paused = run(RUN_ID, UNITY_TEMPLATE, NativeRunStatus::Paused);
    assert!(detect_with(&[snapshot], &[paused], DoctorLiveRun::Running).is_empty());
}

#[test]
fn a_paused_run_of_the_current_step_of_a_live_flow_is_a_live_incident() {
    let snapshot = paused_flow_step(FlowStatus::Running);
    let paused = run(RUN_ID, UNITY_TEMPLATE, NativeRunStatus::Paused);
    let found = detect_with(&[snapshot], &[paused], DoctorLiveRun::Running);
    assert_eq!(found.len(), 1);
    assert!(found[0].live);
}

#[test]
fn a_run_without_a_flow_counts_for_a_day_only() {
    let old = run_started(OTHER_RUN_ID, UNITY_TEMPLATE, TWO_DAYS_AGO);
    assert!(detect_with(&[], &[old], DoctorLiveRun::Running).is_empty());
    let recent = run_started(OTHER_RUN_ID, UNITY_TEMPLATE, LONG_AGO);
    let found = detect_with(&[], &[recent], DoctorLiveRun::Running);
    assert_eq!(found.len(), 1);
    assert!(!found[0].live);
}

#[test]
fn a_run_replaced_by_a_newer_run_is_not_an_incident() {
    let older = run_started("run-old", UNITY_TEMPLATE, LONG_AGO);
    let newer = run_started("run-new", UNITY_TEMPLATE, JUST_NOW);
    let found = detect_with(&[], &[older, newer], DoctorLiveRun::Running);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].run_id.as_deref(), Some("run-new"));
}

#[test]
fn live_flows_come_first_and_the_most_recent_first() {
    let mut stale_flow = flow(
        FlowStatus::Failed,
        IMPLEMENT,
        row(
            StepStatus::Failed,
            Some(StepReason::PipelineFailed),
            LONG_AGO,
        ),
    );
    stale_flow.state.key = "GH-1".to_string();
    stale_flow.state.ticket = serde_json::json!({ "key": "GH-1" });
    let mut older_live = flow(
        FlowStatus::WaitingExternal,
        IMPLEMENT,
        row(
            StepStatus::WaitingExternal,
            Some(StepReason::Budget),
            LONG_AGO,
        ),
    );
    older_live.state.key = "GH-2".to_string();
    older_live.state.ticket = serde_json::json!({ "key": "GH-2" });
    let mut newer_live = flow(
        FlowStatus::WaitingExternal,
        IMPLEMENT,
        row(
            StepStatus::WaitingExternal,
            Some(StepReason::Budget),
            "2026-11-09T11:00:00Z",
        ),
    );
    newer_live.state.key = "GH-3".to_string();
    newer_live.state.ticket = serde_json::json!({ "key": "GH-3" });
    let recent_run = run_started(OTHER_RUN_ID, UNITY_TEMPLATE, JUST_NOW);
    let found = detect_with(
        &[stale_flow, older_live, newer_live],
        &[recent_run],
        DoctorLiveRun::Running,
    );
    let keys: Vec<&str> = found.iter().map(|c| c.key.as_str()).collect();
    assert_eq!(keys[0], "GH-3", "{keys:?}");
    assert_eq!(keys[1], "GH-2", "{keys:?}");
    assert!(!found[2].live && !found[3].live, "{keys:?}");
}

#[test]
fn an_ended_run_is_stuck_only_after_the_grace_and_several_ticks() {
    let snapshot = with_run(
        flow(
            FlowStatus::Running,
            IMPLEMENT,
            row(StepStatus::Running, None, LONG_AGO),
        ),
        IMPLEMENT,
        RUN_ID,
    );
    let ended_long_ago = DoctorLiveRun::Ended(Some(LONG_AGO.to_string()));
    // Seen for 20 seconds: the engine has not had its time to reconcile.
    let at_first_sight = detect_seen(
        std::slice::from_ref(&snapshot),
        &[],
        ended_long_ago.clone(),
        20_000,
    );
    assert!(at_first_sight.is_empty(), "no false step_stuck");
    // The end carries no time: the doctor's own sighting counts, and it is fresh.
    let no_time = detect_seen(
        std::slice::from_ref(&snapshot),
        &[],
        DoctorLiveRun::Ended(None),
        20_000,
    );
    assert!(no_time.is_empty());
    // Seen for more than the grace and ended long ago: stuck.
    let later = detect_seen(&[snapshot], &[], ended_long_ago, 4 * 60_000);
    assert_eq!(later.len(), 1);
    assert_eq!(later[0].trigger, DoctorTrigger::StepStuck);
}

#[test]
fn an_art_run_stopped_on_art_findings_is_not_an_incident() {
    let mut paused = run(OTHER_RUN_ID, ART_TEMPLATE, NativeRunStatus::Paused);
    paused.reason = Some(ART_FINDINGS.to_string());
    paused.started_at = JUST_NOW.to_string();
    paused.ended_at = Some(JUST_NOW.to_string());
    assert!(detect_with(&[], &[paused], DoctorLiveRun::Running).is_empty());
}

#[test]
fn an_art_run_stopped_on_a_passing_fault_is_an_incident() {
    let mut paused = run(OTHER_RUN_ID, ART_TEMPLATE, NativeRunStatus::Paused);
    paused.reason = Some("MCP server pixellab: CONNECT_TIMEOUT".to_string());
    paused.started_at = JUST_NOW.to_string();
    paused.ended_at = Some(JUST_NOW.to_string());
    assert_eq!(detect_with(&[], &[paused], DoctorLiveRun::Running).len(), 1);
}

#[test]
fn a_flow_failed_on_art_findings_is_not_an_incident() {
    let snapshot = with_run(
        flow(
            FlowStatus::Failed,
            IMPLEMENT,
            row(
                StepStatus::Failed,
                Some(StepReason::PipelineFailed),
                LONG_AGO,
            ),
        ),
        IMPLEMENT,
        RUN_ID,
    );
    let mut paused = run(RUN_ID, ART_TEMPLATE, NativeRunStatus::Paused);
    paused.reason = Some(ART_FINDINGS.to_string());
    assert!(detect_with(&[snapshot], &[paused], DoctorLiveRun::Running).is_empty());
}

const BUG_CAPPED: &str = "bug_capped_stop";

#[test]
fn a_ticket_stopped_on_the_cap_of_bugs_is_an_incident_of_bugs_that_do_not_converge() {
    let snapshot = flow(
        FlowStatus::Escalated,
        BUG_CAPPED,
        row(
            StepStatus::Escalated,
            Some(StepReason::BusinessError),
            LONG_AGO,
        ),
    );
    let started = millis_of(LONG_AGO).unwrap();
    let found = detect_with(&[snapshot], &[], DoctorLiveRun::Running);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].trigger, DoctorTrigger::BugsNotConverging);
    assert_eq!(found[0].step.as_deref(), Some(BUG_CAPPED));
    assert_eq!(
        found[0].id,
        format!("{}.{started}", step_incident_id(KEY, BUG_CAPPED, 0))
    );
}

#[test]
fn the_cap_of_bugs_is_an_incident_even_when_the_step_failed_instead_of_escalating() {
    let snapshot = flow(
        FlowStatus::Failed,
        BUG_CAPPED,
        row(
            StepStatus::Failed,
            Some(StepReason::BusinessError),
            LONG_AGO,
        ),
    );
    let found = detect_with(&[snapshot], &[], DoctorLiveRun::Running);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].trigger, DoctorTrigger::BugsNotConverging);
}

#[test]
fn a_later_cap_of_the_same_ticket_has_another_incident_id() {
    let mut later = flow(
        FlowStatus::Escalated,
        BUG_CAPPED,
        row(
            StepStatus::Escalated,
            Some(StepReason::BusinessError),
            LONG_AGO,
        ),
    );
    later.state.started_at = Some(JUST_NOW.to_string());
    let earlier = flow(
        FlowStatus::Escalated,
        BUG_CAPPED,
        row(
            StepStatus::Escalated,
            Some(StepReason::BusinessError),
            LONG_AGO,
        ),
    );
    let first = detect_with(&[earlier], &[], DoctorLiveRun::Running);
    let second = detect_with(&[later], &[], DoctorLiveRun::Running);
    assert_ne!(first[0].id, second[0].id);
}

#[test]
fn a_cap_stop_that_was_aborted_is_no_longer_an_incident() {
    let snapshot = flow(
        FlowStatus::Aborted,
        BUG_CAPPED,
        row(StepStatus::Escalated, Some(StepReason::Aborted), LONG_AGO),
    );
    assert!(detect_with(&[snapshot], &[], DoctorLiveRun::Running).is_empty());
}
