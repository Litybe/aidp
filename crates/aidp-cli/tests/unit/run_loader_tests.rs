use super::*;
use aidp_core::run::{StageState, StageStatus};

fn state_with(stages: &[(&str, StageStatus, Option<u64>)]) -> RunState {
    let mut run_state: RunState = serde_json::from_value(serde_json::json!({
        "run_id": "r1", "pipeline_id": "dev", "run_dir": "/x",
        "pipeline_path": "/p", "workspace_root": "/w", "started_at": "t", "stages": []
    }))
    .unwrap();
    for (name, status, tokens) in stages {
        let mut stage: StageState = serde_json::from_value(serde_json::json!({
            "name": name, "status": "pending", "stage_dir": "/s"
        }))
        .unwrap();
        stage.status = *status;
        stage.tokens_used = *tokens;
        run_state.stages.push(stage);
    }
    run_state
}

#[test]
fn unknown_gate_lists_stage_names_and_known_gate_passes() {
    let run_state = state_with(&[("plan", StageStatus::Success, None)]);
    assert!(unknown_gate_refusal(&run_state, "plan").is_none());
    let refusal = unknown_gate_refusal(&run_state, "nexistepas").unwrap();
    assert!(refusal.diagnostics[0].contains("nexistepas"));
    assert!(refusal.diagnostics[0].contains("plan"));
    assert_eq!(refusal.exit, Some(ExitStatus::Usage));
}
