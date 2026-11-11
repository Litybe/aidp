use super::*;
use aidp_flows::{FlowDef, Frame, FrameKind};
use serde_json::json;
use std::collections::BTreeMap;

const NOW: &str = "2026-11-11T12:00:00Z";
const STEP: &str = "report";
const FLOW_KEY: &str = "GH-7";
const HOUR: i64 = 3_600_000;

fn at(text: &str) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339(text)
        .unwrap()
        .with_timezone(&chrono::Utc)
}

fn definition(id: &str, kind: &str, use_: &str, optional: bool) -> Step {
    let text = format!(
        "id: f\nversion: 1\nsteps:\n  - {{ id: {id}, kind: {kind}, use: {use_}, optional: {optional} }}\n"
    );
    FlowDef::from_yaml(&text).unwrap().steps.remove(0)
}

fn state_of(step: Step, status: StepStatus, reason: Option<&str>) -> FlowState {
    let mut state = FlowState::new(FLOW_KEY, FlowDef::default());
    state.stack.push(Frame {
        kind: FrameKind::Root,
        prefix: String::new(),
        owner: None,
        steps: vec![step.clone()],
        index: 0,
    });
    state.current = Some(step.id.clone());
    state.steps.insert(
        step.id,
        StepState {
            status,
            reason: reason.map(str::to_string),
            ..StepState::default()
        },
    );
    state
}

fn failed_report() -> FlowState {
    state_of(
        definition(STEP, "pipeline", "report-stage", false),
        StepStatus::Failed,
        None,
    )
}

#[test]
fn a_failed_report_step_is_marked_passed_with_a_pull_request_and_a_clean_recheck() {
    assert_eq!(mark_passed(&failed_report(), STEP, true, true), Ok(()));
}

#[test]
fn mark_passed_needs_the_pull_request_and_the_clean_recheck() {
    let state = failed_report();
    assert!(mark_passed(&state, STEP, false, true)
        .unwrap_err()
        .contains("no pull request"));
    assert!(mark_passed(&state, STEP, true, false)
        .unwrap_err()
        .contains("recheck"));
}

#[test]
fn mark_passed_never_decides_a_human_or_a_merge_step() {
    let human = state_of(
        definition("verdict", "human", "noop", false),
        StepStatus::Escalated,
        None,
    );
    assert!(mark_passed(&human, "verdict", true, true)
        .unwrap_err()
        .contains("human"));
    let merge = state_of(
        definition("merge_report", "action", "gh.merge", false),
        StepStatus::Failed,
        None,
    );
    assert!(mark_passed(&merge, "merge_report", true, true)
        .unwrap_err()
        .contains("merges"));
}

#[test]
fn mark_passed_needs_a_report_step_that_failed_and_was_not_rejected() {
    let other = state_of(
        definition("implement", "pipeline", "dev", false),
        StepStatus::Failed,
        None,
    );
    assert!(mark_passed(&other, "implement", true, true)
        .unwrap_err()
        .contains("report or verdict"));
    let running = state_of(
        definition(STEP, "pipeline", "report-stage", false),
        StepStatus::Running,
        None,
    );
    assert!(mark_passed(&running, STEP, true, true).is_err());
    let rejected = state_of(
        definition(STEP, "pipeline", "report-stage", false),
        StepStatus::Escalated,
        Some("rejected"),
    );
    assert!(mark_passed(&rejected, STEP, true, true).is_err());
    assert!(mark_passed(&failed_report(), "elsewhere", true, true)
        .unwrap_err()
        .contains("not the current step"));
}

#[test]
fn an_optional_step_may_be_skipped_and_a_required_one_may_not() {
    let optional = state_of(
        definition("lint", "action", "noop", true),
        StepStatus::Failed,
        None,
    );
    assert_eq!(skip_optional(&optional, "lint"), Ok(()));
    let required = state_of(
        definition("lint", "action", "noop", false),
        StepStatus::Failed,
        None,
    );
    assert!(skip_optional(&required, "lint")
        .unwrap_err()
        .contains("optional"));
    let healthy = state_of(
        definition("lint", "action", "noop", true),
        StepStatus::Running,
        None,
    );
    assert!(skip_optional(&healthy, "lint").is_err());
    let human = state_of(
        definition("ask", "human", "noop", true),
        StepStatus::Escalated,
        None,
    );
    assert!(skip_optional(&human, "ask").unwrap_err().contains("human"));
}

fn merge_state(status: StepStatus, approved: bool) -> FlowState {
    let mut state = state_of(
        definition("merge", "action", "gh.merge", false),
        status,
        None,
    );
    if approved {
        let mut outputs = BTreeMap::new();
        outputs.insert("decision".to_string(), json!("approve"));
        outputs.insert("ok".to_string(), json!(true));
        state.steps.insert(
            "approval".to_string(),
            StepState {
                status: StepStatus::Done,
                outputs,
                ..StepState::default()
            },
        );
    }
    state
}

#[test]
fn the_merge_step_runs_again_only_with_the_approval_held_and_a_mergeable_pull_request() {
    let approved = merge_state(StepStatus::Failed, true);
    assert_eq!(retry_merge(&approved, "merge", true), Ok(()));
    let unapproved = merge_state(StepStatus::Failed, false);
    assert!(retry_merge(&unapproved, "merge", true)
        .unwrap_err()
        .contains("approval"));
    assert!(retry_merge(&approved, "merge", false)
        .unwrap_err()
        .contains("mergeable"));
    let running = merge_state(StepStatus::Running, true);
    assert!(retry_merge(&running, "merge", true).is_err());
    assert!(retry_merge(&failed_report(), STEP, true)
        .unwrap_err()
        .contains("gh.merge"));
}

#[test]
fn a_marked_passed_step_is_not_an_approval() {
    let mut state = merge_state(StepStatus::Failed, false);
    let mut outputs = BTreeMap::new();
    outputs.insert("ok".to_string(), json!(true));
    outputs.insert("marked_passed".to_string(), json!(true));
    state.steps.insert(
        "report".to_string(),
        StepState {
            status: StepStatus::Done,
            outputs,
            ..StepState::default()
        },
    );
    assert!(!holds_approval(&state));
}

#[test]
fn the_pull_request_merges_only_when_the_status_says_so() {
    assert!(pr_mergeable(Some(&json!({ "mergeable": "mergeable" }))));
    assert!(!pr_mergeable(Some(&json!({ "mergeable": "conflicting" }))));
    assert!(!pr_mergeable(Some(&json!({}))));
    assert!(!pr_mergeable(None));
}

#[test]
fn a_running_step_is_reconciled_and_nothing_else() {
    let running = state_of(
        definition("implement", "pipeline", "dev", false),
        StepStatus::Running,
        None,
    );
    assert_eq!(reconcile(&running, "implement"), Ok(()));
    assert!(reconcile(&failed_report(), STEP).is_err());
}

#[test]
fn the_budget_is_nudged_for_a_step_that_waits_for_it() {
    let waiting = state_of(
        definition("implement", "pipeline", "dev", false),
        StepStatus::WaitingExternal,
        Some("budget"),
    );
    assert_eq!(nudge_budget(&waiting, "implement"), Ok(false));
    let ceiling = state_of(
        definition("implement", "pipeline", "dev", false),
        StepStatus::WaitingHuman,
        Some("ticket-budget"),
    );
    assert_eq!(nudge_budget(&ceiling, "implement"), Ok(true));
    let elsewhere = state_of(
        definition("implement", "pipeline", "dev", false),
        StepStatus::WaitingExternal,
        Some("resource:unity-editor"),
    );
    assert!(nudge_budget(&elsewhere, "implement").is_err());
}

#[test]
fn only_a_compile_gate_step_gets_a_longer_wait() {
    let gate = state_of(
        definition("refresh", "action", "unity.refresh", false),
        StepStatus::Failed,
        None,
    );
    assert_eq!(extend_wait(&gate, "refresh"), Ok(()));
    let other = state_of(
        definition("refresh", "action", "noop", false),
        StepStatus::Failed,
        None,
    );
    assert!(extend_wait(&other, "refresh")
        .unwrap_err()
        .contains("unity.refresh"));
}

#[test]
fn the_fallback_model_must_be_set() {
    assert_eq!(fallback_model(Some(" sonnet ")), Ok("sonnet".to_string()));
    assert!(fallback_model(None).is_err());
    assert!(fallback_model(Some("  ")).is_err());
}

fn unity(running: bool, heartbeat_at: &str) -> Value {
    json!({ "editor": { "running": running, "heartbeat": { "at": heartbeat_at } } })
}

#[test]
fn a_modal_is_dismissed_only_when_the_editor_runs_and_its_heartbeat_is_dead() {
    let now = at(NOW);
    assert_eq!(
        editor_blocked(&unity(true, "2026-11-11T11:50:00Z"), now),
        Ok(())
    );
    assert!(editor_blocked(&unity(true, "2026-11-11T11:59:50Z"), now)
        .unwrap_err()
        .contains("alive"));
    assert!(editor_blocked(&unity(false, "2026-11-11T11:00:00Z"), now)
        .unwrap_err()
        .contains("never opens"));
    assert!(editor_blocked(&json!({ "editor": { "running": true } }), now).is_err());
}

#[test]
fn the_lock_of_a_live_holder_is_released_when_its_heartbeat_died_for_stuck_minutes() {
    assert_eq!(lock_release(true, "GH-1", Some("done"), None, 10), Ok(()));
    assert_eq!(
        lock_release(false, "GH-1", Some("running"), Some(11 * 60), 10),
        Ok(())
    );
    assert!(
        lock_release(false, "GH-1", Some("running"), Some(5 * 60), 10)
            .unwrap_err()
            .contains("not terminal")
    );
    assert!(lock_release(false, "GH-1", Some("running"), None, 10).is_err());
}

#[test]
fn a_worktree_is_pruned_only_for_a_flow_over_for_long_and_idle() {
    let now = 100 * HOUR;
    let long_ago = Some(now - 25 * HOUR);
    assert!(worktree_prunable(
        FlowStatus::Done,
        long_ago,
        now,
        24,
        false
    ));
    assert!(worktree_prunable(
        FlowStatus::Failed,
        long_ago,
        now,
        24,
        false
    ));
    let recent = Some(now - 23 * HOUR);
    assert!(!worktree_prunable(FlowStatus::Done, recent, now, 24, false));
    assert!(!worktree_prunable(
        FlowStatus::Running,
        long_ago,
        now,
        24,
        false
    ));
    assert!(!worktree_prunable(
        FlowStatus::Done,
        long_ago,
        now,
        24,
        true
    ));
    assert!(!worktree_prunable(FlowStatus::Done, None, now, 24, false));
}

#[test]
fn only_meta_files_both_sides_added_are_resolved() {
    let clean = "M  Assets/A.cs\nAA Assets/Folder.meta\nAA Assets/Other.meta\n";
    assert_eq!(
        meta_conflicts(clean),
        Ok(vec![
            "Assets/Folder.meta".to_string(),
            "Assets/Other.meta".to_string()
        ])
    );
    let code = "AA Assets/Folder.meta\nUU Assets/Player.cs\n";
    assert_eq!(
        meta_conflicts(code),
        Err(vec!["Assets/Player.cs".to_string()])
    );
    let modified = "UU Assets/Folder.meta\n";
    assert_eq!(
        meta_conflicts(modified),
        Err(vec!["Assets/Folder.meta".to_string()])
    );
    assert_eq!(meta_conflicts("M  Assets/A.cs\n"), Ok(Vec::new()));
}
