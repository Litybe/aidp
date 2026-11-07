use super::*;
use aidp_core::run::StageState;

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
fn summary_value_counts_stages_and_tokens() {
    let run_state = state_with(&[
        ("plan", StageStatus::Success, Some(10)),
        ("implement", StageStatus::Cancel, Some(5)),
        ("pr", StageStatus::Pending, None),
        ("verify", StageStatus::InProgress, None),
    ]);
    let value = run_summary_value(&run_state);
    assert_eq!(value["counts"]["total"], 4);
    assert_eq!(value["counts"]["done"], 1);
    assert_eq!(value["counts"]["cancel"], 1);
    assert_eq!(value["counts"]["pending"], 1);
    assert_eq!(value["counts"]["in_progress"], 1);
    assert_eq!(value["tokens"], 15);
    assert_eq!(value["stages"][0]["name"], "plan");
}

#[test]
fn compact_view_is_one_line_and_text_is_unchanged() {
    let run_state = state_with(&[("plan", StageStatus::Success, None)]);
    let output = render_run_view(Some(Format::JsonCompact), &run_state, "md".into());
    let crate::commands::Payload::Json { value, format } = output.payload else {
        panic!("expected json payload");
    };
    let line = crate::output::render_value(format.unwrap(), &value).unwrap();
    assert!(!line.contains('\n'));
    let text = render_run_view(None, &run_state, "md".into());
    assert!(matches!(text.payload, crate::commands::Payload::Text(ref body) if body == "md"));
}
