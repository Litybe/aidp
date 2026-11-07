//! Two flows contending for the Unity editor lock, through the real executor and fake sessions.

use super::*;

const UNITY_FLOW: &str = "unity-flow";
const EDITOR: &str = "unity-editor:p1";
const WAITING_REASON: &str = "resource:unity-editor";

fn start_unity_flow(engine: &Arc<FlowEngine>, key: &str) {
    engine.start(ticket(key), Some(UNITY_FLOW), None).unwrap();
}

fn holder_key(engine: &Arc<FlowEngine>) -> Option<String> {
    engine
        .executor()
        .resources()
        .snapshot(EDITOR)
        .0
        .map(|holder| holder.flow_key)
}

fn waiting_reason(engine: &Arc<FlowEngine>, key: &str) -> Option<String> {
    engine
        .get(key)
        .ok()?
        .steps
        .get(STEP_DEVELOP)
        .and_then(|step| step.reason.clone())
}

fn lock_events(harness: &Harness) -> Vec<Value> {
    harness
        .bus
        .recent(500, 0)
        .into_iter()
        .filter(|event| event["type"] == crate::unity_lock::EVENT_LOCK_CHANGED)
        .collect()
}

#[test]
fn it_second_flow_waits_for_the_editor_and_resumes_when_the_first_ends() {
    let harness = Harness::new("unity_lock_two_flows");
    let engine = harness.engine();

    start_unity_flow(&engine, "LOC-1");
    wait_step(&engine, "LOC-1", STEP_DEVELOP, StepStatus::Running);
    start_unity_flow(&engine, "LOC-2");
    wait_step(&engine, "LOC-2", STEP_DEVELOP, StepStatus::WaitingExternal);

    assert_eq!(holder_key(&engine).as_deref(), Some("LOC-1"));
    assert_eq!(
        waiting_reason(&engine, "LOC-2").as_deref(),
        Some(WAITING_REASON)
    );
    assert_eq!(harness.sessions.launch_count(), 1);
    let holder = engine.executor().resources().snapshot(EDITOR).0.unwrap();
    assert_eq!(holder.ticket_key, "LOC-1");
    assert_eq!(holder.step_id, STEP_DEVELOP);

    harness
        .sessions
        .finish(&harness.sessions.session_of(0), RUN_DONE);
    tick(&engine);

    wait_step(&engine, "LOC-2", STEP_DEVELOP, StepStatus::Running);
    assert_eq!(holder_key(&engine).as_deref(), Some("LOC-2"));
    assert_eq!(harness.sessions.launch_count(), 2);
    wait_for("the first flow to finish", || {
        run_status(&engine, "LOC-1") == FlowStatus::Done
    });
    assert!(!lock_events(&harness).is_empty());
    engine.abort("LOC-2", ACTOR).unwrap();
}

#[test]
fn it_waiters_are_served_in_arrival_order() {
    let harness = Harness::new("unity_lock_fifo");
    let engine = harness.engine();
    start_unity_flow(&engine, "LOC-1");
    wait_step(&engine, "LOC-1", STEP_DEVELOP, StepStatus::Running);
    start_unity_flow(&engine, "LOC-2");
    wait_step(&engine, "LOC-2", STEP_DEVELOP, StepStatus::WaitingExternal);
    start_unity_flow(&engine, "LOC-3");
    wait_step(&engine, "LOC-3", STEP_DEVELOP, StepStatus::WaitingExternal);

    harness
        .sessions
        .finish(&harness.sessions.session_of(0), RUN_DONE);
    tick(&engine);

    wait_step(&engine, "LOC-2", STEP_DEVELOP, StepStatus::Running);
    tick(&engine);
    assert_eq!(
        step_status(&engine, "LOC-3", STEP_DEVELOP),
        Some(StepStatus::WaitingExternal)
    );
    assert_eq!(holder_key(&engine).as_deref(), Some("LOC-2"));
    engine.abort("LOC-2", ACTOR).unwrap();
    engine.abort("LOC-3", ACTOR).unwrap();
}

#[test]
fn it_abort_of_the_holder_frees_the_editor_for_the_waiter() {
    let harness = Harness::new("unity_lock_abort_holder");
    let engine = harness.engine();
    start_unity_flow(&engine, "LOC-1");
    wait_step(&engine, "LOC-1", STEP_DEVELOP, StepStatus::Running);
    start_unity_flow(&engine, "LOC-2");
    wait_step(&engine, "LOC-2", STEP_DEVELOP, StepStatus::WaitingExternal);

    engine.abort("LOC-1", ACTOR).unwrap();

    wait_step(&engine, "LOC-2", STEP_DEVELOP, StepStatus::Running);
    assert_eq!(holder_key(&engine).as_deref(), Some("LOC-2"));
    engine.abort("LOC-2", ACTOR).unwrap();
    assert_eq!(holder_key(&engine), None);
}

#[test]
fn it_abort_of_a_waiter_leaves_the_queue_and_the_holder_alone() {
    let harness = Harness::new("unity_lock_abort_waiter");
    let engine = harness.engine();
    start_unity_flow(&engine, "LOC-1");
    wait_step(&engine, "LOC-1", STEP_DEVELOP, StepStatus::Running);
    start_unity_flow(&engine, "LOC-2");
    wait_step(&engine, "LOC-2", STEP_DEVELOP, StepStatus::WaitingExternal);

    engine.abort("LOC-2", ACTOR).unwrap();

    let (holder, waiting) = engine.executor().resources().snapshot(EDITOR);
    assert_eq!(holder.unwrap().flow_key, "LOC-1");
    assert!(waiting.is_empty());
    engine.abort("LOC-1", ACTOR).unwrap();
}

#[test]
fn it_failed_pipeline_releases_the_editor() {
    let harness = Harness::new("unity_lock_failed");
    let engine = harness.engine();
    start_unity_flow(&engine, "LOC-1");
    wait_step(&engine, "LOC-1", STEP_DEVELOP, StepStatus::Running);
    assert_eq!(holder_key(&engine).as_deref(), Some("LOC-1"));

    harness
        .sessions
        .finish(&harness.sessions.session_of(0), "failed");
    tick(&engine);

    wait_step(&engine, "LOC-1", STEP_DEVELOP, StepStatus::Failed);
    assert_eq!(holder_key(&engine), None);
}

#[test]
fn it_skipping_a_waiting_step_leaves_the_queue() {
    let harness = Harness::new("unity_lock_skip");
    let engine = harness.engine();
    start_unity_flow(&engine, "LOC-1");
    wait_step(&engine, "LOC-1", STEP_DEVELOP, StepStatus::Running);
    start_unity_flow(&engine, "LOC-2");
    wait_step(&engine, "LOC-2", STEP_DEVELOP, StepStatus::WaitingExternal);

    engine.skip("LOC-2", STEP_DEVELOP, ACTOR).unwrap();

    assert!(engine.executor().resources().snapshot(EDITOR).1.is_empty());
    assert_eq!(holder_key(&engine).as_deref(), Some("LOC-1"));
    engine.abort("LOC-1", ACTOR).unwrap();
}

#[test]
fn it_restart_rebuilds_the_holder_and_the_waiter_does_not_take_the_editor() {
    let harness = Harness::new("unity_lock_restart");
    let engine = harness.engine();
    start_unity_flow(&engine, "LOC-1");
    wait_step(&engine, "LOC-1", STEP_DEVELOP, StepStatus::Running);
    start_unity_flow(&engine, "LOC-2");
    wait_step(&engine, "LOC-2", STEP_DEVELOP, StepStatus::WaitingExternal);
    wait_for("the engine to go idle", || engine.executor().is_idle());

    let restarted = harness.engine();
    assert_eq!(
        holder_key(&restarted),
        None,
        "nothing is held before resume"
    );
    resume_all(&restarted);

    assert_eq!(holder_key(&restarted).as_deref(), Some("LOC-1"));
    assert_eq!(
        step_status(&restarted, "LOC-2", STEP_DEVELOP),
        Some(StepStatus::WaitingExternal)
    );
    assert_eq!(harness.sessions.launch_count(), 1);
}

#[test]
fn it_restart_with_a_lost_session_leaves_no_phantom_lock() {
    let harness = Harness::new("unity_lock_phantom");
    let engine = harness.engine();
    start_unity_flow(&engine, "LOC-1");
    wait_step(&engine, "LOC-1", STEP_DEVELOP, StepStatus::Running);
    wait_for("the engine to go idle", || engine.executor().is_idle());
    harness.sessions.forget(&harness.sessions.session_of(0));

    let restarted = harness.engine();
    resume_all(&restarted);

    wait_step(&restarted, "LOC-1", STEP_DEVELOP, StepStatus::Failed);
    assert_eq!(holder_key(&restarted), None);
}

#[test]
fn it_manual_release_frees_the_editor_and_the_waiter_takes_it() {
    let harness = Harness::new("unity_lock_release");
    let engine = harness.engine();
    start_unity_flow(&engine, "LOC-1");
    wait_step(&engine, "LOC-1", STEP_DEVELOP, StepStatus::Running);
    start_unity_flow(&engine, "LOC-2");
    wait_step(&engine, "LOC-2", STEP_DEVELOP, StepStatus::WaitingExternal);

    let released = engine.executor().release_resource(EDITOR).unwrap();

    assert_eq!(released.flow_key, "LOC-1");
    // The waiter is woken on a worker thread that saves the step as Running before it takes the
    // lock: wait for the hand-over itself and for that worker to be done, not only for the status.
    wait_step(&engine, "LOC-2", STEP_DEVELOP, StepStatus::Running);
    wait_for("LOC-2 to hold the editor", || {
        holder_key(&engine).as_deref() == Some("LOC-2")
    });
    wait_for("the wake-up worker to finish", || {
        engine.executor().is_idle()
    });
    // The former holder still runs but is not claimed again while another flow holds the editor.
    tick(&engine);
    assert_eq!(holder_key(&engine).as_deref(), Some("LOC-2"));
    assert!(engine
        .executor()
        .release_resource("unity-editor:other")
        .is_none());
    let journal = engine.journal("LOC-1").unwrap();
    assert!(journal
        .iter()
        .any(|line| line.line.contains("released by hand")));
    engine.abort("LOC-1", ACTOR).unwrap();
    engine.abort("LOC-2", ACTOR).unwrap();
}

#[test]
fn it_lock_change_events_carry_the_holder_and_the_waiters() {
    let harness = Harness::new("unity_lock_events");
    let engine = harness.engine();
    start_unity_flow(&engine, "LOC-1");
    wait_step(&engine, "LOC-1", STEP_DEVELOP, StepStatus::Running);
    start_unity_flow(&engine, "LOC-2");
    wait_step(&engine, "LOC-2", STEP_DEVELOP, StepStatus::WaitingExternal);

    let events = lock_events(&harness);

    let taken = &events[0]["data"];
    assert_eq!(taken["resource"], EDITOR);
    assert_eq!(taken["held"], true);
    assert_eq!(taken["holder"]["ticketKey"], "LOC-1");
    let queued = &events[events.len() - 1]["data"];
    assert_eq!(queued["waiting"], json!(["LOC-2"]));
    engine.abort("LOC-1", ACTOR).unwrap();
    engine.abort("LOC-2", ACTOR).unwrap();
}
