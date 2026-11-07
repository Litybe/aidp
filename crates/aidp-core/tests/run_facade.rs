use aidp_core::run::{
    anchor_not_persisted, approved_gates, carry_over_task_outcomes, ensure_knowledge_snapshot,
    feature_key, fold_workflow_result, init, init_with, load_in, new_run_id, overall_status,
    parse_auto_approve_gates, parse_resume_anchor, plan_accounting_anomalies, read_resume_anchor,
    reconcile, reconcile_with, record_plan_schedules, record_resume, record_resume_with,
    render_progress, render_summary, resolve_approve_gates, run_dir_for_in, save, save_state,
    stage_complete_noop, stage_skip, ComplexityBucket, GateDecision, GateDecisionKind, LoopState,
    ResumeAnchor, RunCost, RunPorts, RunState, StageState, StageStatus, StageUsage, UsageSource,
    WorkflowStatus, INPUT_JIRA_KEY, INPUT_TARGET, RUN_STATE_FILE, WORKFLOW_RESULT_FILE,
};

#[test]
fn every_public_item_of_the_run_facade_is_importable() {
    let type_names = [
        std::any::type_name::<ComplexityBucket>(),
        std::any::type_name::<GateDecision>(),
        std::any::type_name::<GateDecisionKind>(),
        std::any::type_name::<LoopState>(),
        std::any::type_name::<ResumeAnchor>(),
        std::any::type_name::<RunCost>(),
        std::any::type_name::<RunPorts>(),
        std::any::type_name::<RunState>(),
        std::any::type_name::<StageState>(),
        std::any::type_name::<StageStatus>(),
        std::any::type_name::<StageUsage>(),
        std::any::type_name::<UsageSource>(),
    ];
    assert!(type_names.iter().all(|name| !name.is_empty()));

    let constants = [
        INPUT_JIRA_KEY,
        INPUT_TARGET,
        RUN_STATE_FILE,
        WORKFLOW_RESULT_FILE,
        WorkflowStatus::AwaitingGate.as_str(),
    ];
    assert!(constants.iter().all(|value| !value.is_empty()));

    let _ = anchor_not_persisted;
    let _ = approved_gates;
    let _ = carry_over_task_outcomes;
    let _ = ensure_knowledge_snapshot;
    let _ = feature_key;
    let _ = fold_workflow_result;
    let _ = init;
    let _ = init_with;
    let _ = load_in;
    let _ = new_run_id;
    let _ = overall_status;
    let _ = parse_auto_approve_gates;
    let _ = parse_resume_anchor;
    let _ = plan_accounting_anomalies;
    let _ = read_resume_anchor;
    let _ = reconcile;
    let _ = reconcile_with;
    let _ = record_plan_schedules;
    let _ = record_resume;
    let _ = record_resume_with;
    let _ = render_progress;
    let _ = render_summary;
    let _ = resolve_approve_gates;
    let _ = run_dir_for_in;
    let _ = save;
    let _ = save_state;
    let _ = stage_complete_noop;
    let _ = stage_skip;
}
