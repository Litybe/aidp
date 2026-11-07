use super::*;
use crate::models::run_state::StageState;

fn pending_stage(name: &str) -> StageState {
    StageState {
        name: name.into(),
        status: StageStatus::Pending,
        ..Default::default()
    }
}

fn three_pending_stages() -> RunState {
    RunState {
        stages: ["context", "plan", "implement"]
            .into_iter()
            .map(pending_stage)
            .collect(),
        ..Default::default()
    }
}

const WORKFLOW_RESULT_DONE: &str = include_str!("../fixtures/workflow_result_done.json");
const RUN_STATE_RECONCILED: &str = include_str!("../fixtures/run_state_reconciled.json");

fn workflow_result() -> serde_json::Value {
    serde_json::from_str(WORKFLOW_RESULT_DONE).unwrap()
}

#[test]
fn fold_is_pure() {
    let state = three_pending_stages();
    let folded = fold_workflow_result(&state, &workflow_result(), &[]);
    let status_of = |name: &str| {
        folded
            .stages
            .iter()
            .find(|s| s.name == name)
            .map(|s| s.status)
    };
    assert_eq!(status_of("context"), Some(StageStatus::Success));
    assert_eq!(status_of("plan"), Some(StageStatus::Success));
    assert_eq!(status_of("implement"), Some(StageStatus::Cancel));
    assert_eq!(folded.last_workflow_status.as_deref(), Some("done"));
    assert_eq!(
        folded.stages[0].summary.as_deref(),
        Some("context gathered")
    );
}

#[test]
fn fold_leaves_input_state_unchanged() {
    let state = three_pending_stages();
    let before = serde_json::to_value(&state).unwrap();
    let _ = fold_workflow_result(&state, &workflow_result(), &[]);
    assert_eq!(serde_json::to_value(&state).unwrap(), before);
}

/// Folding the done fixture yields exactly the checked-in reconciled state.
#[test]
fn fold_of_the_done_fixture_matches_the_reconciled_state_byte_for_byte() {
    let folded = fold_workflow_result(&three_pending_stages(), &workflow_result(), &[]);
    let pretty = format!(
        "{}
",
        serde_json::to_string_pretty(&folded).unwrap()
    );
    assert_eq!(pretty, RUN_STATE_RECONCILED);
}
