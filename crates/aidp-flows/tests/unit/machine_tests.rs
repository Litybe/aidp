use super::*;
use serde_json::json;

const MANUAL_LABEL: &str = "manual-0";
const MANUAL_PUBLISH_LABEL: &str = "manual-0-publish";
const MANUAL_PUBLISH_FIELD: &str = "manual_publish";
const PUBLISH_FIELD: &str = "publish";
const MANUAL_FAILURE: &str = "manual publication refused";
const OUTCOME_VALUE: &str = "value";
const HUMAN_DEPLOY: &str = include_str!("../fixtures/human_deploy.yaml");
const NESTED_HUMAN_DEPLOY: &str = include_str!("../fixtures/nested_human_deploy.yaml");
const DEPLOY_APPROVAL: &str = "deploy_approval";
const DEPLOY_STEP: &str = "deploy";
const DEPLOY_DEV_ACTION: &str = "deploy.dev";
const DEPLOY_RUN_ID: &str = "run_id";
const NESTED_APPROVAL: &str = "release/deploy_approval";
const NESTED_DEPLOY: &str = "release/deploy";
const REJECT_ACTOR: &str = "alexis";
const WAITING_REASON: &str = "awaiting_approval";
const ENVIRONMENT_APPROVERS: &str = "approvers";
const ENVIRONMENT_URL: &str = "url";
const DEPLOYMENT_URL: &str = "https://github.com/acme/widgets/actions/runs/9";
const DEPLOYMENT_RUN: u64 = 9;
const BASIC: &str = include_str!("../fixtures/basic.yaml");
const LOOP_MAX: &str = include_str!("../fixtures/loop_max.yaml");
const AT: &str = "2026-10-02T08:00:00Z";
const LATER: &str = "2026-10-02T09:30:00Z";
const KEY: &str = "LOC-1";
const SOURCE_LOCAL: &str = "local";
const SOURCE_JIRA: &str = "jira";
const NOOP: &str = "noop";
const FAIL: &str = "fail";
const PIPELINE_TEMPLATE: &str = "dev";
const CI_WAIT: &str = "ci@build/wait-ci";
const PUBLISH: &str = include_str!("../fixtures/publish.yaml");
const PUBLISH_LOOP: &str = include_str!("../fixtures/publish_loop.yaml");
const STEP_BUILD: &str = "build";
const STEP_REVIEW: &str = "review";
const STEP_AFTER: &str = "after";
const STEP_FIX: &str = "fix";
const STEP_CHECK: &str = "check";
const PUBLISH_PUSH: &str = "publish@build/git_push";
const PUBLISH_PR: &str = "publish@build/pr_create";
const PUBLISH_REFERENCE: &str = "publish@build";
const CI_WATCH: &str = "ci@build/ci_watch";
const LOOP_PUSH: &str = "publish@fix/git_push";
const PUBLISH_FAILURE: &str = "exit 5: gh api graphql failed";
const OUTPUT_GREEN: &str = "green";
const ENTRY_MERGE: &str = include_str!("../fixtures/entry_merge.yaml");
const ENTRY_MERGE_NAME: &str = "merge";
const ENTRY_UNKNOWN: &str = "nowhere";
const STEP_CHECKOUT: &str = "checkout";
const STEP_TEST: &str = "test";
const STEP_TEST_LOOP: &str = "test_loop";
const STEP_READY: &str = "ready";
const STEP_MERGE: &str = "merge";
const OUTPUT_PASSED: &str = "passed";

type Stepped = (FlowState, Vec<Effect>);

fn def_of(text: &str) -> FlowDef {
    FlowDef::from_yaml(text).unwrap()
}

fn action(id: &str, name: &str) -> Step {
    let mut step = Step::new(id, StepKind::Action);
    step.use_ = Some(name.to_string());
    step
}

fn pipeline(id: &str) -> Step {
    let mut step = Step::new(id, StepKind::Pipeline);
    step.use_ = Some(PIPELINE_TEMPLATE.to_string());
    step
}

fn def_with(steps: Vec<Step>) -> FlowDef {
    FlowDef {
        id: "test".to_string(),
        version: 1,
        steps,
        ..FlowDef::default()
    }
}

fn start_with(def: &FlowDef, source: &str) -> Stepped {
    let event = Event::Start {
        ticket: json!({"source": source, "key": KEY}),
        at: AT.to_string(),
        entry: None,
    };
    advance(def, FlowState::new(KEY, def.clone()), event).unwrap()
}

fn start(def: &FlowDef) -> Stepped {
    start_with(def, SOURCE_JIRA)
}

fn send(def: &FlowDef, stepped: Stepped, event: Event) -> Stepped {
    advance(def, stepped.0, event).unwrap()
}

fn finish_with(step: &str, iteration: u32, outputs: &[(&str, Value)]) -> Event {
    let outputs: BTreeMap<String, Value> = outputs
        .iter()
        .map(|(key, value)| (key.to_string(), value.clone()))
        .collect();
    Event::ActionFinished {
        effect_id: effect_id(step, iteration, EffectName::RunAction.as_str()),
        outcome: ActionOutcome::Ok(outputs),
        at: AT.to_string(),
    }
}

fn finish(step: &str, iteration: u32) -> Event {
    finish_with(step, iteration, &[])
}

fn fail_with(step: &str, iteration: u32, outcome: ActionOutcome) -> Event {
    Event::ActionFinished {
        effect_id: effect_id(step, iteration, EffectName::RunAction.as_str()),
        outcome,
        at: AT.to_string(),
    }
}

fn pipeline_done(step: &str, iteration: u32) -> Event {
    Event::PipelineFinished {
        effect_id: effect_id(step, iteration, EffectName::StartPipeline.as_str()),
        run_status: PipelineRunStatus::Done.as_str().to_string(),
        outputs: BTreeMap::new(),
        at: AT.to_string(),
    }
}

fn decide(step: &str, decision: &str) -> Event {
    Event::HumanDecision {
        step: step.to_string(),
        decision: decision.to_string(),
        actor: "alexis".to_string(),
        at: LATER.to_string(),
        report: None,
    }
}

fn effect_ids(stepped: &Stepped) -> Vec<String> {
    stepped.1.iter().map(|effect| effect.id.clone()).collect()
}

fn has_effect(stepped: &Stepped, id: &str) -> bool {
    effect_ids(stepped).iter().any(|candidate| candidate == id)
}

fn run_action_count(stepped: &Stepped) -> usize {
    stepped
        .1
        .iter()
        .filter(|effect| matches!(effect.kind, EffectKind::RunAction { .. }))
        .count()
}

fn step_state<'a>(stepped: &'a Stepped, id: &str) -> &'a StepState {
    &stepped.0.steps[id]
}

fn rejection(def: &FlowDef, state: FlowState, event: Event) -> RejectReason {
    advance(def, state, event)
        .unwrap_err()
        .reject_reason()
        .expect("a rejected event")
}

#[test]
fn start_emits_started_and_the_first_action() {
    let def = def_of(BASIC);

    let stepped = start(&def);

    assert!(has_effect(&stepped, "prepare:0:run_action"));
    assert_eq!(stepped.0.current.as_deref(), Some("prepare"));
    assert_eq!(stepped.0.status, FlowStatus::Running);
    assert!(stepped.1.iter().any(|effect| matches!(
        effect.kind,
        EffectKind::Emit {
            event: FlowEvent::Started,
            ..
        }
    )));
}

#[test]
fn start_twice_is_rejected() {
    let def = def_of(BASIC);
    let stepped = start(&def);
    let event = Event::Start {
        ticket: json!({}),
        at: AT.to_string(),
        entry: None,
    };

    assert_eq!(
        rejection(&def, stepped.0, event),
        RejectReason::AlreadyStarted
    );
}

#[test]
fn event_before_start_is_rejected() {
    let def = def_of(BASIC);
    let state = FlowState::new(KEY, def.clone());

    assert_eq!(
        rejection(&def, state, Event::Pause { at: AT.to_string() }),
        RejectReason::NotStarted
    );
}

#[test]
fn full_run_walks_action_pipeline_wait_human_action() {
    let def = def_of(BASIC);
    let mut stepped = start(&def);
    stepped = send(&def, stepped, finish("prepare", 0));
    assert!(has_effect(&stepped, "build:0:start_pipeline"));
    assert_eq!(
        step_state(&stepped, "build").phase.as_deref(),
        Some("build")
    );

    stepped = send(&def, stepped, pipeline_done("build", 0));
    assert_eq!(stepped.0.inserted_ci, vec!["ci@build"]);
    assert!(has_effect(&stepped, "ci@build/wait-ci:0:start_wait"));
    assert_eq!(stepped.0.status, FlowStatus::WaitingExternal);
    assert_eq!(stepped.0.current.as_deref(), Some(CI_WAIT));

    let wait_event = Event::WaitSatisfied {
        effect_id: effect_id(CI_WAIT, 0, EffectName::StartWait.as_str()),
        outputs: BTreeMap::new(),
        at: AT.to_string(),
    };
    stepped = send(&def, stepped, wait_event);
    assert_eq!(step_state(&stepped, "ci@build").status, StepStatus::Done);
    assert!(has_effect(&stepped, "review:0:create_inbox"));
    assert_eq!(stepped.0.status, FlowStatus::WaitingHuman);

    stepped = send(&def, stepped, decide("review", "approve"));
    assert!(has_effect(&stepped, "review:0:resolve_inbox"));
    assert!(has_effect(&stepped, "publish:0:run_action"));
    stepped = send(&def, stepped, finish("publish", 0));
    stepped = send(&def, stepped, finish("local-only", 0));

    assert_eq!(stepped.0.status, FlowStatus::Done);
    assert!(stepped.0.ended_at.is_some());
    assert!(stepped.0.stack.is_empty());
}

#[test]
fn ci_block_is_not_inserted_without_a_ci_block_definition() {
    let mut pushing = pipeline("build");
    pushing.pushes = true;
    let def = def_with(vec![pushing, action("after", NOOP)]);

    let mut stepped = start(&def);
    stepped = send(&def, stepped, pipeline_done("build", 0));

    assert!(stepped.0.inserted_ci.is_empty());
    assert!(has_effect(&stepped, "after:0:run_action"));
}

#[test]
fn when_false_skips_the_step_with_a_reason() {
    let mut def = def_of(BASIC);
    def.steps
        .iter_mut()
        .find(|step| step.id == STEP_REVIEW)
        .unwrap()
        .choices
        .push(HumanDecision::Ko.as_str().to_string());
    let mut stepped = start(&def);
    stepped = send(&def, stepped, finish("prepare", 0));
    stepped = send(&def, stepped, pipeline_done("build", 0));
    let wait_event = Event::WaitSatisfied {
        effect_id: effect_id(CI_WAIT, 0, EffectName::StartWait.as_str()),
        outputs: BTreeMap::new(),
        at: AT.to_string(),
    };
    stepped = send(&def, stepped, wait_event);

    stepped = send(
        &def,
        stepped,
        decide(STEP_REVIEW, HumanDecision::Ko.as_str()),
    );

    let publish = step_state(&stepped, "publish");
    assert_eq!(publish.status, StepStatus::Skipped);
    assert_eq!(
        publish.reason.as_deref(),
        Some(StepReason::WhenFalse.as_str())
    );
    assert!(publish.detail.as_deref().unwrap().contains("is false"));
    assert!(has_effect(&stepped, "local-only:0:run_action"));
}

#[test]
fn skip_on_local_skips_only_local_tickets() {
    let def = def_with(vec![{
        let mut step = action("tracker-only", NOOP);
        step.skip_on = vec![SOURCE_LOCAL.to_string()];
        step
    }]);

    let local = start_with(&def, SOURCE_LOCAL);
    let remote = start_with(&def, SOURCE_JIRA);

    assert_eq!(
        step_state(&local, "tracker-only").status,
        StepStatus::Skipped
    );
    assert_eq!(
        step_state(&local, "tracker-only").reason.as_deref(),
        Some(StepReason::SkipOn.as_str())
    );
    assert_eq!(local.0.status, FlowStatus::Done);
    assert_eq!(run_action_count(&remote), 1);
}

#[test]
fn on_error_warn_records_a_warning_and_continues() {
    let mut flaky = action("flaky", FAIL);
    flaky.on_error = OnError::Warn;
    let def = def_with(vec![flaky, action("after", NOOP)]);
    let mut stepped = start(&def);

    stepped = send(
        &def,
        stepped,
        fail_with("flaky", 0, ActionOutcome::TechnicalError("boom".into())),
    );

    let flaky_state = step_state(&stepped, "flaky");
    assert_eq!(flaky_state.status, StepStatus::Failed);
    assert_eq!(flaky_state.warnings, vec!["boom"]);
    assert!(has_effect(&stepped, "after:0:run_action"));
    assert_eq!(stepped.0.status, FlowStatus::Running);
}

#[test]
fn technical_error_fails_and_business_error_escalates() {
    let def = def_with(vec![action("a", FAIL), action("b", NOOP)]);

    let technical = send(
        &def,
        start(&def),
        fail_with("a", 0, ActionOutcome::TechnicalError("boom".into())),
    );
    let business = send(
        &def,
        start(&def),
        fail_with("a", 0, ActionOutcome::BusinessError("no go".into())),
    );

    assert_eq!(technical.0.status, FlowStatus::Failed);
    assert_eq!(
        step_state(&technical, "a").reason.as_deref(),
        Some(StepReason::TechnicalError.as_str())
    );
    assert_eq!(business.0.status, FlowStatus::Escalated);
    assert_eq!(step_state(&business, "a").status, StepStatus::Escalated);
    assert_eq!(run_action_count(&technical), 0);
}

/// A skipped outcome marks the step Skipped and the flow moves on.
#[test]
fn skipped_outcome_marks_the_step_skipped_and_continues() {
    let def = def_with(vec![action("a", NOOP), action("b", NOOP)]);
    let stepped = send(
        &def,
        start(&def),
        fail_with(
            "a",
            0,
            ActionOutcome::Skipped {
                reason: "local_ticket".into(),
            },
        ),
    );

    let state = step_state(&stepped, "a");
    assert_eq!(state.status, StepStatus::Skipped);
    assert_eq!(
        state.reason.as_deref(),
        Some(StepReason::ActionSkipped.as_str())
    );
    assert_eq!(state.detail.as_deref(), Some("local_ticket"));
    assert!(has_effect(&stepped, "b:0:run_action"));
    assert_eq!(stepped.0.status, FlowStatus::Running);
}

/// A waiting outcome parks the step on the external condition and the flow with it.
#[test]
fn waiting_outcome_parks_the_step_and_the_flow() {
    let def = def_with(vec![action("a", NOOP), action("b", NOOP)]);
    let stepped = send(
        &def,
        start(&def),
        fail_with(
            "a",
            0,
            ActionOutcome::WaitingExternal {
                reason: "worktree_cap".into(),
                outputs: BTreeMap::new(),
            },
        ),
    );

    let state = step_state(&stepped, "a");
    assert_eq!(state.status, StepStatus::WaitingExternal);
    assert_eq!(
        state.reason.as_deref(),
        Some(StepReason::ActionWaiting.as_str())
    );
    assert_eq!(state.detail.as_deref(), Some("worktree_cap"));
    assert!(!has_effect(&stepped, "b:0:run_action"));
    assert_eq!(stepped.0.status, FlowStatus::WaitingExternal);
}

#[test]
fn retry_reruns_a_failed_step_with_a_new_effect_id() {
    let def = def_with(vec![action("a", FAIL), action("b", NOOP)]);
    let mut stepped = send(
        &def,
        start(&def),
        fail_with("a", 0, ActionOutcome::TechnicalError("boom".into())),
    );

    let stale = fail_with("a", 0, ActionOutcome::Ok(BTreeMap::new()));
    stepped = send(
        &def,
        stepped,
        Event::Retry {
            step: "a".to_string(),
            at: AT.to_string(),
        },
    );

    assert!(has_effect(&stepped, "a:1:run_action"));
    assert_eq!(stepped.0.status, FlowStatus::Running);
    assert_eq!(
        rejection(&def, stepped.0.clone(), stale),
        RejectReason::StaleEffect
    );
    stepped = send(&def, stepped, finish("a", 1));
    assert!(has_effect(&stepped, "b:0:run_action"));
}

#[test]
fn skip_moves_past_a_failed_step() {
    let def = def_with(vec![action("a", FAIL), action("b", NOOP)]);
    let mut stepped = send(
        &def,
        start(&def),
        fail_with("a", 0, ActionOutcome::TechnicalError("boom".into())),
    );

    stepped = send(
        &def,
        stepped,
        Event::Skip {
            step: "a".to_string(),
            at: AT.to_string(),
        },
    );

    assert_eq!(
        step_state(&stepped, "a").reason.as_deref(),
        Some(StepReason::ManualSkip.as_str())
    );
    assert!(has_effect(&stepped, "b:0:run_action"));
}

#[test]
fn mark_passed_moves_past_a_failed_step_and_keeps_the_flow_going() {
    let def = def_with(vec![action("a", FAIL), action("b", NOOP)]);
    let mut stepped = send(
        &def,
        start(&def),
        fail_with("a", 0, ActionOutcome::TechnicalError("boom".into())),
    );

    stepped = send(
        &def,
        stepped,
        Event::MarkPassed {
            step: "a".to_string(),
            at: AT.to_string(),
        },
    );

    let passed = step_state(&stepped, "a");
    assert_eq!(passed.status, StepStatus::Done);
    assert_eq!(passed.outputs.get("marked_passed"), Some(&json!(true)));
    assert!(has_effect(&stepped, "b:0:run_action"));
}

#[test]
fn mark_passed_of_a_step_that_did_not_fail_is_rejected() {
    let def = def_with(vec![action("a", NOOP)]);
    let stepped = start(&def);

    let event = Event::MarkPassed {
        step: "a".to_string(),
        at: AT.to_string(),
    };

    assert_eq!(rejection(&def, stepped.0, event), RejectReason::WrongState);
}

#[test]
fn the_optional_flag_of_a_step_is_read_and_defaults_to_false() {
    let def = def_of(
        "id: f
version: 1
steps:
  - { id: a, kind: action, use: noop, optional: true }
  - { id: b, kind: action, use: noop }
",
    );

    assert!(def.steps[0].optional);
    assert!(!def.steps[1].optional);
}

#[test]
fn retry_of_a_step_that_did_not_fail_is_rejected() {
    let def = def_with(vec![action("a", NOOP)]);
    let stepped = start(&def);

    let event = Event::Retry {
        step: "a".to_string(),
        at: AT.to_string(),
    };

    assert_eq!(rejection(&def, stepped.0, event), RejectReason::WrongState);
}

#[test]
fn loop_stops_when_until_is_satisfied() {
    let def = def_of(LOOP_MAX);
    let mut stepped = start(&def);
    assert!(has_effect(&stepped, "fix:0:run_action"));

    stepped = send(&def, stepped, finish("fix", 0));
    stepped = send(
        &def,
        stepped,
        finish_with("check", 0, &[("green", json!(true))]),
    );

    assert_eq!(step_state(&stepped, "fix-loop").status, StepStatus::Done);
    assert_eq!(stepped.0.status, FlowStatus::Done);
}

#[test]
fn loop_until_on_a_string_output_stops_at_the_second_round() {
    let mut def = def_of(LOOP_MAX);
    def.steps[0].until = Some("steps.check.outputs.verdict == 'done'".to_string());
    def.steps[0].max = Some(5);
    let mut stepped = start(&def);

    stepped = send(&def, stepped, finish("fix", 0));
    stepped = send(
        &def,
        stepped,
        finish_with("check", 0, &[("verdict", json!("need-fix"))]),
    );
    assert!(has_effect(&stepped, "fix:1:run_action"));
    stepped = send(&def, stepped, finish("fix", 1));
    stepped = send(
        &def,
        stepped,
        finish_with("check", 1, &[("verdict", json!("done"))]),
    );

    let loop_state = step_state(&stepped, "fix-loop");
    assert_eq!(loop_state.status, StepStatus::Done);
    assert_eq!(loop_state.iteration, 2);
    assert_eq!(stepped.0.status, FlowStatus::Done);
}

#[test]
fn when_on_a_boolean_output_runs_the_step_only_for_false() {
    for (passed, runs) in [(false, true), (true, false)] {
        let mut after = action("after", NOOP);
        after.when = Some("steps.test.outputs.passed == false".to_string());
        let def = def_with(vec![action("test", NOOP), after]);
        let mut stepped = start(&def);

        stepped = send(
            &def,
            stepped,
            finish_with("test", 0, &[("passed", json!(passed))]),
        );

        assert_eq!(has_effect(&stepped, "after:0:run_action"), runs);
        if !runs {
            assert_eq!(step_state(&stepped, "after").status, StepStatus::Skipped);
        }
    }
}

#[test]
fn loop_repeats_then_escalates_when_max_is_reached() {
    let def = def_of(LOOP_MAX);
    let red = [("green", json!(false))];
    let mut stepped = start(&def);

    stepped = send(&def, stepped, finish("fix", 0));
    stepped = send(&def, stepped, finish_with("check", 0, &red));
    assert!(has_effect(&stepped, "fix:1:run_action"));
    assert_eq!(step_state(&stepped, "fix-loop").iteration, 2);

    stepped = send(&def, stepped, finish("fix", 1));
    stepped = send(&def, stepped, finish_with("check", 1, &red));

    let loop_state = step_state(&stepped, "fix-loop");
    assert_eq!(loop_state.status, StepStatus::Escalated);
    assert_eq!(loop_state.iteration, 2);
    assert_eq!(
        loop_state.reason.as_deref(),
        Some(StepReason::MaxIterations.as_str())
    );
    assert_eq!(stepped.0.status, FlowStatus::Escalated);
}

#[test]
fn loop_with_human_step_waits_for_each_decision() {
    let mut review = Step::new("ask", StepKind::Human);
    review.choices = vec![
        HumanDecision::Ok.as_str().to_string(),
        HumanDecision::Ko.as_str().to_string(),
    ];
    let mut loop_step = Step::new("rounds", StepKind::Loop);
    loop_step.until = Some("steps.ask.outputs.ok == true".to_string());
    loop_step.body = vec![review];
    let def = def_with(vec![loop_step]);

    let mut stepped = start(&def);
    assert_eq!(stepped.0.status, FlowStatus::WaitingHuman);
    stepped = send(&def, stepped, decide("ask", HumanDecision::Ko.as_str()));
    assert_eq!(stepped.0.status, FlowStatus::WaitingHuman);
    assert!(has_effect(&stepped, "ask:1:create_inbox"));
    stepped = send(&def, stepped, decide("ask", HumanDecision::Ok.as_str()));

    assert_eq!(stepped.0.status, FlowStatus::Done);
}

#[test]
fn human_decision_on_a_step_not_waiting_is_rejected() {
    let def = def_of(BASIC);
    let stepped = start(&def);

    let reason = rejection(&def, stepped.0, decide("review", "approve"));

    assert_eq!(reason, RejectReason::WrongState);
}

#[test]
fn human_decision_on_an_undefined_step_is_unknown() {
    let def = def_of(BASIC);
    let stepped = start(&def);

    let reason = rejection(&def, stepped.0, decide("ghost", "approve"));

    assert_eq!(reason, RejectReason::UnknownStep);
}

#[test]
fn human_decision_outside_the_choices_is_rejected() {
    let def = def_with(vec![{
        let mut ask = Step::new("ask", StepKind::Human);
        ask.choices = vec!["approve".to_string()];
        ask
    }]);
    let stepped = start(&def);

    let reason = rejection(&def, stepped.0, decide("ask", "maybe"));

    assert_eq!(reason, RejectReason::InvalidChoice);
}

#[test]
fn human_decision_journals_actor_and_time_and_stores_outputs() {
    let def = def_with(vec![Step::new("ask", StepKind::Human)]);
    let mut stepped = start(&def);

    stepped = send(&def, stepped, decide("ask", "approve"));

    let journal = stepped
        .1
        .iter()
        .find_map(|effect| match &effect.kind {
            EffectKind::Journal { at, line } if line.contains("alexis") => {
                Some((at.clone(), line.clone()))
            }
            _ => None,
        })
        .expect("journal line with the actor");
    assert_eq!(journal.0, LATER);
    assert!(journal.1.contains(LATER));
    let outputs = &step_state(&stepped, "ask").outputs;
    assert_eq!(outputs[OUTPUT_ACTOR], json!("alexis"));
    assert_eq!(outputs[OUTPUT_DECISION], json!("approve"));
    assert_eq!(outputs[OUTPUT_OK], json!(true));
}

#[test]
fn pause_lets_the_running_step_finish_then_stops() {
    let def = def_with(vec![action("a", NOOP), action("b", NOOP)]);
    let mut stepped = start(&def);

    stepped = send(&def, stepped, Event::Pause { at: AT.to_string() });
    assert_eq!(stepped.0.status, FlowStatus::Pausing);

    stepped = send(&def, stepped, finish("a", 0));
    assert_eq!(stepped.0.status, FlowStatus::Paused);
    assert_eq!(run_action_count(&stepped), 0);

    stepped = send(&def, stepped, Event::Resume { at: AT.to_string() });
    assert!(has_effect(&stepped, "b:0:run_action"));
    assert_eq!(stepped.0.status, FlowStatus::Running);
}

#[test]
fn resume_without_pause_is_rejected() {
    let def = def_with(vec![action("a", NOOP)]);
    let stepped = start(&def);

    let reason = rejection(&def, stepped.0, Event::Resume { at: AT.to_string() });

    assert_eq!(reason, RejectReason::NotPaused);
}

#[test]
fn abort_cancels_the_running_pipeline_session() {
    let def = def_with(vec![pipeline("build"), action("after", NOOP)]);
    let mut stepped = start(&def);

    stepped = send(&def, stepped, Event::Abort { at: AT.to_string() });

    assert!(has_effect(&stepped, "build:0:cancel_session"));
    assert_eq!(stepped.0.status, FlowStatus::Aborted);
    assert_eq!(
        step_state(&stepped, "build").reason.as_deref(),
        Some(StepReason::Aborted.as_str())
    );
    let again = rejection(&def, stepped.0, Event::Abort { at: AT.to_string() });
    assert_eq!(again, RejectReason::NotActive);
}

#[test]
fn abort_closes_a_pending_inbox_entry() {
    let def = def_with(vec![Step::new("ask", StepKind::Human)]);
    let mut stepped = start(&def);

    stepped = send(&def, stepped, Event::Abort { at: AT.to_string() });

    assert!(has_effect(&stepped, "ask:0:resolve_inbox"));
}

#[test]
fn insufficient_budget_waits_externally_until_retry() {
    let def = def_with(vec![pipeline("build")]);
    let mut stepped = start(&def);

    stepped = send(
        &def,
        stepped,
        Event::BudgetInsufficient {
            step: "build".to_string(),
            at: AT.to_string(),
        },
    );

    assert_eq!(stepped.0.status, FlowStatus::WaitingExternal);
    assert_eq!(
        step_state(&stepped, "build").reason.as_deref(),
        Some(StepReason::Budget.as_str())
    );
    stepped = send(
        &def,
        stepped,
        Event::Retry {
            step: "build".to_string(),
            at: AT.to_string(),
        },
    );
    assert!(has_effect(&stepped, "build:1:start_pipeline"));
    assert_eq!(stepped.0.status, FlowStatus::Running);
}

#[test]
fn orphaned_pipeline_fails_the_step_with_reason_orphaned() {
    let def = def_with(vec![pipeline("build")]);
    let stepped = start(&def);

    let stepped = send(
        &def,
        stepped,
        Event::PipelineOrphaned {
            effect_id: effect_id("build", 0, EffectName::StartPipeline.as_str()),
            at: AT.to_string(),
        },
    );

    assert_eq!(
        step_state(&stepped, "build").reason.as_deref(),
        Some(StepReason::Orphaned.as_str())
    );
    assert_eq!(stepped.0.status, FlowStatus::Failed);
}

#[test]
fn pipeline_run_that_is_not_done_escalates_or_fails() {
    let def = def_with(vec![pipeline("build")]);
    let cases = [
        ("failed", StepStatus::Failed),
        ("paused", StepStatus::Escalated),
    ];

    for (run_status, expected) in cases {
        let event = Event::PipelineFinished {
            effect_id: effect_id("build", 0, EffectName::StartPipeline.as_str()),
            run_status: run_status.to_string(),
            outputs: BTreeMap::new(),
            at: AT.to_string(),
        };
        let stepped = send(&def, start(&def), event);
        assert_eq!(
            step_state(&stepped, "build").status,
            expected,
            "{run_status}"
        );
    }
}

#[test]
fn manual_request_emits_an_effect_with_its_own_id() {
    let def = def_with(vec![action("a", NOOP)]);
    let mut stepped = start(&def);

    stepped = send(
        &def,
        stepped,
        Event::Manual {
            request: ManualRequest::Action {
                name: NOOP.to_string(),
                with: BTreeMap::new(),
            },
            at: AT.to_string(),
        },
    );

    assert!(has_effect(&stepped, "manual-0:0:run_action"));
    assert_eq!(stepped.0.manual_count, 1);
    assert_eq!(stepped.0.current.as_deref(), Some("a"));
}

#[test]
fn effect_ids_are_deterministic() {
    let def = def_of(BASIC);

    let first = effect_ids(&start(&def));
    let second = effect_ids(&start(&def));

    assert_eq!(first, second);
}

#[test]
fn state_survives_a_json_round_trip_mid_run() {
    let def = def_of(LOOP_MAX);
    let mut stepped = start(&def);
    stepped = send(&def, stepped, finish("fix", 0));

    let text = serde_json::to_string(&stepped.0).unwrap();
    let restored: FlowState = serde_json::from_str(&text).unwrap();
    let resumed = advance(&def, restored, finish("check", 0)).unwrap();

    assert!(has_effect(&resumed, "fix:1:run_action"));
}

/// Asserts the string form and the serde form of a value are identical.
fn assert_wire<T: serde::Serialize>(value: T, expected: &str) {
    assert_eq!(serde_json::to_value(value).unwrap(), json!(expected));
}

#[test]
fn effect_names_have_stable_wire_strings() {
    let expected = [
        (EffectName::RunAction, "run_action"),
        (EffectName::StartPipeline, "start_pipeline"),
        (EffectName::CancelSession, "cancel_session"),
        (EffectName::CreateInbox, "create_inbox"),
        (EffectName::ResolveInbox, "resolve_inbox"),
        (EffectName::StartWait, "start_wait"),
        (EffectName::Journal, "journal"),
        (EffectName::Emit, "emit"),
    ];
    for (name, text) in expected {
        assert_eq!(name.as_str(), text);
        assert_wire(name, text);
    }
}

#[test]
fn step_reasons_have_stable_wire_strings() {
    let expected = [
        (StepReason::WhenFalse, "when-false"),
        (StepReason::SkipOn, "skip-on"),
        (StepReason::ManualSkip, "manual-skip"),
        (StepReason::ExpressionError, "expression-error"),
        (StepReason::TechnicalError, "technical-error"),
        (StepReason::BusinessError, "business-error"),
        (StepReason::Orphaned, "orphaned"),
        (StepReason::Budget, "budget"),
        (StepReason::MaxIterations, "max-iterations"),
        (StepReason::RunStatus, "run-status"),
        (StepReason::PipelineFailed, "pipeline-failed"),
        (StepReason::Aborted, "aborted"),
        (StepReason::Rejected, "rejected"),
    ];
    for (reason, text) in expected {
        assert_eq!(reason.as_str(), text);
        assert_wire(reason, text);
    }
}

#[test]
fn pipeline_run_statuses_have_stable_wire_strings_and_parse_back() {
    let expected = [
        (PipelineRunStatus::Done, "done"),
        (PipelineRunStatus::Failed, "failed"),
        (PipelineRunStatus::Orphaned, "orphaned"),
        (PipelineRunStatus::Cancelled, "cancelled"),
    ];
    for (status, text) in expected {
        assert_eq!(status.as_str(), text);
        assert_wire(status, text);
        assert_eq!(PipelineRunStatus::parse(text), Some(status));
    }
    assert_eq!(PipelineRunStatus::parse("unknown"), None);
}

#[test]
fn human_decisions_have_stable_wire_strings_and_positive_set() {
    let expected = [
        (HumanDecision::Approve, "approve", true),
        (HumanDecision::Reject, "reject", false),
        (HumanDecision::Click, "click", true),
        (HumanDecision::Ok, "ok", true),
        (HumanDecision::Ko, "ko", false),
    ];
    for (decision, text, positive) in expected {
        assert_eq!(decision.as_str(), text);
        assert_wire(decision, text);
        assert_eq!(HumanDecision::parse(text), Some(decision));
        assert_eq!(decision.is_ok(), positive);
    }
    assert_eq!(HumanDecision::parse("custom"), None);
}

#[test]
fn flow_label_and_manual_prefix_keep_their_values() {
    assert_eq!(FLOW_LABEL, "flow");
    assert_eq!(MANUAL_PREFIX, "manual-");
}

/// The ids of the run-action and start-pipeline effects, in emission order.
fn work_effects(stepped: &Stepped) -> Vec<String> {
    stepped
        .1
        .iter()
        .filter(|effect| {
            matches!(
                effect.kind,
                EffectKind::RunAction { .. } | EffectKind::StartPipeline { .. }
            )
        })
        .map(|effect| effect.id.clone())
        .collect()
}

/// A pushing step is followed by the publish block, then the ci block; a non-pushing one by neither.
#[test]
fn the_publish_block_runs_after_a_push_and_before_the_ci_block() {
    let def = def_of(PUBLISH);
    let mut stepped = start(&def);
    assert_eq!(
        work_effects(&stepped),
        vec![effect_id(STEP_BUILD, 0, EffectName::StartPipeline.as_str())]
    );

    stepped = send(&def, stepped, pipeline_done(STEP_BUILD, 0));
    assert_eq!(
        work_effects(&stepped),
        vec![effect_id(PUBLISH_PUSH, 0, EffectName::RunAction.as_str())]
    );
    stepped = send(&def, stepped, finish(PUBLISH_PUSH, 0));
    assert_eq!(
        work_effects(&stepped),
        vec![effect_id(PUBLISH_PR, 0, EffectName::RunAction.as_str())]
    );
    stepped = send(&def, stepped, finish(PUBLISH_PR, 0));
    assert_eq!(
        step_state(&stepped, PUBLISH_REFERENCE).status,
        StepStatus::Done
    );
    assert_eq!(
        work_effects(&stepped),
        vec![effect_id(CI_WATCH, 0, EffectName::RunAction.as_str())]
    );
    assert_eq!(stepped.0.inserted_ci, vec!["ci@build"]);
    stepped = send(&def, stepped, finish(CI_WATCH, 0));
    assert_eq!(
        work_effects(&stepped),
        vec![effect_id(
            STEP_REVIEW,
            0,
            EffectName::StartPipeline.as_str()
        )]
    );

    stepped = send(&def, stepped, pipeline_done(STEP_REVIEW, 0));
    assert_eq!(
        work_effects(&stepped),
        vec![effect_id(STEP_AFTER, 0, EffectName::RunAction.as_str())]
    );
}

/// A failed publish fails the run on the publish step: the ci block never starts.
#[test]
fn a_failed_publish_fails_the_run_before_the_ci_block() {
    let def = def_of(PUBLISH);
    let mut stepped = start(&def);
    stepped = send(&def, stepped, pipeline_done(STEP_BUILD, 0));

    stepped = send(
        &def,
        stepped,
        fail_with(
            PUBLISH_PUSH,
            0,
            ActionOutcome::TechnicalError(PUBLISH_FAILURE.to_string()),
        ),
    );

    assert_eq!(stepped.0.status, FlowStatus::Failed);
    assert_eq!(stepped.0.current.as_deref(), Some(PUBLISH_PUSH));
    assert_eq!(
        step_state(&stepped, PUBLISH_PUSH).status,
        StepStatus::Failed
    );
    assert!(!stepped.0.steps.contains_key(CI_WATCH));
    assert!(work_effects(&stepped).is_empty());
}

/// A pushing step in a loop is published on every iteration, never twice in one.
#[test]
fn a_pushing_step_in_a_loop_is_published_once_per_iteration() {
    let def = def_of(PUBLISH_LOOP);
    let mut stepped = start(&def);
    for iteration in 0..2 {
        stepped = send(&def, stepped, pipeline_done(STEP_FIX, iteration));
        assert_eq!(
            work_effects(&stepped),
            vec![effect_id(
                LOOP_PUSH,
                iteration,
                EffectName::RunAction.as_str()
            )]
        );
        stepped = send(&def, stepped, finish(LOOP_PUSH, iteration));
        assert_eq!(
            work_effects(&stepped),
            vec![effect_id(
                STEP_CHECK,
                iteration,
                EffectName::RunAction.as_str()
            )]
        );
        let green = iteration == 1;
        stepped = send(
            &def,
            stepped,
            finish_with(STEP_CHECK, iteration, &[(OUTPUT_GREEN, json!(green))]),
        );
    }
    assert_eq!(stepped.0.status, FlowStatus::Done);
}

/// Only a successfully completed manual pipeline with explicit publishing emits a push.
#[test]
fn manual_pipeline_publishes_only_done_with_opt_in() {
    for publish in [false, true] {
        for status in [
            PipelineRunStatus::Done,
            PipelineRunStatus::Failed,
            PipelineRunStatus::Orphaned,
            PipelineRunStatus::Cancelled,
        ] {
            let definition = def_with(vec![action(STEP_BUILD, NOOP)]);
            let requested = send(
                &definition,
                start(&definition),
                Event::Manual {
                    request: ManualRequest::Pipeline {
                        template: PIPELINE_TEMPLATE.to_string(),
                        publish,
                        with: BTreeMap::new(),
                    },
                    at: AT.to_string(),
                },
            );
            assert_eq!(requested.0.manual_publish.contains(MANUAL_LABEL), publish);
            let restored: FlowState =
                serde_json::from_value(serde_json::to_value(&requested.0).unwrap()).unwrap();
            let completed = advance(
                &definition,
                restored,
                Event::PipelineFinished {
                    effect_id: effect_id(MANUAL_LABEL, 0, EffectName::StartPipeline.as_str()),
                    run_status: status.as_str().to_string(),
                    outputs: BTreeMap::new(),
                    at: LATER.to_string(),
                },
            )
            .unwrap();
            let pushes: Vec<_> = completed.1.iter().filter(|effect| matches!(&effect.kind, EffectKind::RunAction { name, .. } if name == PUBLISH_ACTION)).collect();
            assert_eq!(
                pushes.len(),
                usize::from(publish && status == PipelineRunStatus::Done)
            );
            assert!(completed.0.manual_publish.is_empty());
            assert_eq!(completed.0.current.as_deref(), Some(STEP_BUILD));
            if let Some(effect) = pushes.first() {
                assert_eq!(
                    effect.id,
                    effect_id(MANUAL_PUBLISH_LABEL, 0, EffectName::RunAction.as_str())
                );
                assert!(
                    matches!(&effect.kind, EffectKind::RunAction { step, with, .. } if step == MANUAL_PUBLISH_LABEL && with.is_empty())
                );
            }
            assert!(completed.1.iter().any(|effect| matches!(&effect.kind, EffectKind::Journal {line, ..} if line == &format!("manual pipeline {} ended {}", effect_id(MANUAL_LABEL, 0, EffectName::StartPipeline.as_str()), status.as_str()))));
            let repeated = send(&definition, completed, pipeline_done(MANUAL_LABEL, 0));
            assert!(!repeated.1.iter().any(|effect| matches!(&effect.kind, EffectKind::RunAction {name, ..} if name == PUBLISH_ACTION)));
        }
    }
}

/// Old persisted states and manual requests retain their opt-out publishing default.
#[test]
fn manual_publish_defaults_allow_legacy_state_and_requests() {
    let mut state = serde_json::to_value(FlowState::new(KEY, FlowDef::default())).unwrap();
    state.as_object_mut().unwrap().remove(MANUAL_PUBLISH_FIELD);
    let restored: FlowState = serde_json::from_value(state).unwrap();
    assert!(restored.manual_publish.is_empty());
    let request = ManualRequest::Pipeline {
        template: PIPELINE_TEMPLATE.to_string(),
        publish: false,
        with: BTreeMap::new(),
    };
    let mut legacy = serde_json::to_value(&request).unwrap();
    legacy.as_object_mut().unwrap().remove(PUBLISH_FIELD);
    assert_eq!(
        serde_json::from_value::<ManualRequest>(legacy).unwrap(),
        request
    );
}

/// Existing manual requests can complete and publish after the main flow has ended.
#[test]
fn manual_completions_remain_valid_in_terminal_flows() {
    for status in [FlowStatus::Done, FlowStatus::Aborted] {
        let definition = def_with(Vec::new());
        let mut begun = start(&definition);
        begun.0.status = status;
        let requested = send(
            &definition,
            begun,
            Event::Manual {
                request: ManualRequest::Pipeline {
                    template: PIPELINE_TEMPLATE.to_string(),
                    publish: true,
                    with: BTreeMap::new(),
                },
                at: AT.to_string(),
            },
        );
        let completed = send(&definition, requested, pipeline_done(MANUAL_LABEL, 0));
        assert_eq!(completed.0.status, status);
        assert!(has_effect(
            &completed,
            &effect_id(MANUAL_PUBLISH_LABEL, 0, EffectName::RunAction.as_str())
        ));
        let pushed = send(&definition, completed, finish(MANUAL_PUBLISH_LABEL, 0));
        assert_eq!(pushed.0.status, status);
    }
}

/// A manual action failure retains the refusal reason in the journal.
#[test]
fn manual_action_errors_are_visible_without_failing_the_flow() {
    let definition = def_with(vec![action(STEP_BUILD, NOOP)]);
    let completed = send(
        &definition,
        start(&definition),
        fail_with(
            MANUAL_PUBLISH_LABEL,
            0,
            ActionOutcome::TechnicalError(MANUAL_FAILURE.to_string()),
        ),
    );
    assert_eq!(completed.0.status, FlowStatus::Running);
    assert!(completed.1.iter().any(|effect| matches!(&effect.kind, EffectKind::Journal {line, ..} if line == &format!("manual effect {} failed: {MANUAL_FAILURE}", effect_id(MANUAL_PUBLISH_LABEL, 0, EffectName::RunAction.as_str())))));
}

/// Rejection parks flat and expanded gates, including gates configured to warn on errors.
#[test]
fn rejected_human_gate_never_launches_deployment_or_allows_skip() {
    for (fixture, gate, deploy) in [
        (HUMAN_DEPLOY, DEPLOY_APPROVAL, DEPLOY_STEP),
        (NESTED_HUMAN_DEPLOY, NESTED_APPROVAL, NESTED_DEPLOY),
    ] {
        let definition = def_of(fixture);
        let denied = send(
            &definition,
            start(&definition),
            decide(gate, HumanDecision::Reject.as_str()),
        );
        assert_eq!(denied.0.status, FlowStatus::Escalated);
        assert_eq!(denied.0.current.as_deref(), Some(gate));
        let state = &denied.0.steps[gate];
        assert_eq!(state.status, StepStatus::Escalated);
        assert_eq!(state.reason.as_deref(), Some(StepReason::Rejected.as_str()));
        assert_eq!(
            state.detail.as_deref(),
            Some(format!("{REJECTED_DETAIL} {REJECT_ACTOR}").as_str())
        );
        assert_eq!(state.outputs[OUTPUT_ACTOR], REJECT_ACTOR);
        assert_eq!(
            state.outputs[OUTPUT_DECISION],
            HumanDecision::Reject.as_str()
        );
        assert_eq!(state.outputs[OUTPUT_OK], false);
        assert!(!denied.0.steps.contains_key(deploy));
        assert!(!denied.1.iter().any(|effect| matches!(&effect.kind, EffectKind::RunAction {name, ..} if name == DEPLOY_DEV_ACTION)));
        assert!(has_effect(
            &denied,
            &effect_id(gate, 0, EffectName::ResolveInbox.as_str())
        ));
        assert_eq!(
            rejection(
                &definition,
                denied.0.clone(),
                Event::Skip {
                    step: gate.to_string(),
                    at: LATER.to_string()
                }
            ),
            RejectReason::WrongState
        );
        let retried = send(
            &definition,
            denied,
            Event::Retry {
                step: gate.to_string(),
                at: LATER.to_string(),
            },
        );
        assert_eq!(retried.0.status, FlowStatus::WaitingHuman);
        assert_eq!(retried.0.steps[gate].iteration, 1);
        assert!(retried.0.steps[gate].outputs.is_empty());
        assert!(has_effect(
            &retried,
            &effect_id(gate, 1, EffectName::CreateInbox.as_str())
        ));
        let approved = send(
            &definition,
            retried,
            decide(gate, HumanDecision::Approve.as_str()),
        );
        assert!(has_effect(
            &approved,
            &effect_id(deploy, 0, EffectName::RunAction.as_str())
        ));
    }
}

/// Pending deployment reports survive both retry and completion without another approval gate.
#[test]
fn pending_deployment_keeps_run_and_approvers_for_same_run_watch() {
    let definition = def_with(vec![action(DEPLOY_STEP, DEPLOY_DEV_ACTION)]);
    let outputs = BTreeMap::from([
        (DEPLOY_RUN_ID.to_string(), json!(DEPLOYMENT_RUN)),
        (ENVIRONMENT_URL.to_string(), json!(DEPLOYMENT_URL)),
        (ENVIRONMENT_APPROVERS.to_string(), json!([REJECT_ACTOR])),
    ]);
    let pending = send(
        &definition,
        start(&definition),
        fail_with(
            DEPLOY_STEP,
            0,
            ActionOutcome::WaitingExternal {
                reason: WAITING_REASON.to_string(),
                outputs: outputs.clone(),
            },
        ),
    );
    assert_eq!(pending.0.status, FlowStatus::WaitingExternal);
    assert_eq!(pending.0.steps[DEPLOY_STEP].outputs, outputs);
    let retried = send(
        &definition,
        pending.clone(),
        Event::Retry {
            step: DEPLOY_STEP.to_string(),
            at: LATER.to_string(),
        },
    );
    assert_eq!(retried.0.steps[DEPLOY_STEP].outputs, outputs);
    let resumed = send(&definition, pending, finish(DEPLOY_STEP, 0));
    assert_eq!(resumed.0.status, FlowStatus::Done);
}

/// A confirmed retry after failure starts afresh and cannot inherit a terminal run identifier.
#[test]
fn failed_deployment_retry_clears_the_old_run() {
    let definition = def_with(vec![action(DEPLOY_STEP, DEPLOY_DEV_ACTION)]);
    let outputs = BTreeMap::from([(DEPLOY_RUN_ID.to_string(), json!(DEPLOYMENT_RUN))]);
    let pending = send(
        &definition,
        start(&definition),
        fail_with(
            DEPLOY_STEP,
            0,
            ActionOutcome::WaitingExternal {
                reason: WAITING_REASON.to_string(),
                outputs,
            },
        ),
    );
    let failed = send(
        &definition,
        pending,
        fail_with(
            DEPLOY_STEP,
            0,
            ActionOutcome::BusinessError(WAITING_REASON.to_string()),
        ),
    );
    let retried = send(
        &definition,
        failed,
        Event::Retry {
            step: DEPLOY_STEP.to_string(),
            at: LATER.to_string(),
        },
    );
    assert!(retried.0.steps[DEPLOY_STEP].outputs.is_empty());
    assert!(has_effect(
        &retried,
        &effect_id(DEPLOY_STEP, 1, EffectName::RunAction.as_str())
    ));
}

/// Alternate decision endpoints cannot bypass the declared approval choices.
#[test]
fn deploy_gate_refuses_click_and_verdict_decisions() {
    let definition = def_of(HUMAN_DEPLOY);
    let waiting = start(&definition);
    for decision in [HumanDecision::Click, HumanDecision::Ok, HumanDecision::Ko] {
        assert_eq!(
            rejection(
                &definition,
                waiting.0.clone(),
                decide(DEPLOY_APPROVAL, decision.as_str())
            ),
            RejectReason::InvalidChoice
        );
    }
    assert_eq!(
        rejection(
            &definition,
            waiting.0,
            Event::Skip {
                step: DEPLOY_APPROVAL.to_string(),
                at: AT.to_string()
            }
        ),
        RejectReason::WrongState
    );
}

/// Legacy waiting outcomes deserialize without the newly preserved report map.
#[test]
fn waiting_outcome_deserialization_defaults_legacy_outputs() {
    let outcome = ActionOutcome::WaitingExternal {
        reason: WAITING_REASON.to_string(),
        outputs: BTreeMap::new(),
    };
    let mut legacy = serde_json::to_value(&outcome).unwrap();
    legacy
        .get_mut(OUTCOME_VALUE)
        .unwrap()
        .as_object_mut()
        .unwrap()
        .remove(CTX_OUTPUTS);
    assert_eq!(
        serde_json::from_value::<ActionOutcome>(legacy).unwrap(),
        outcome
    );
}

#[test]
fn with_placeholders_read_step_outputs_and_inputs() {
    let mut comment = action("comment", NOOP);
    comment.with = BTreeMap::from([
        (
            "body".to_string(),
            json!("PR #{{ steps.test.outputs.pr }}: {{ steps.test.outputs.verdict }}"),
        ),
        ("number".to_string(), json!("{{ steps.test.outputs.pr }}")),
    ]);
    let def = def_with(vec![action("test", NOOP), comment]);
    let mut stepped = start(&def);

    stepped = send(
        &def,
        stepped,
        finish_with("test", 0, &[("pr", json!(7)), ("verdict", json!("done"))]),
    );

    let with = stepped
        .1
        .iter()
        .find_map(|effect| match &effect.kind {
            EffectKind::RunAction { step, with, .. } if step == "comment" => Some(with.clone()),
            _ => None,
        })
        .expect("comment effect");
    assert_eq!(with["body"], json!("PR #7: done"));
    assert_eq!(with["number"], json!(7));
}

#[test]
fn busy_resource_waits_externally_until_retry() {
    let def = def_with(vec![pipeline("build")]);
    let mut stepped = start(&def);

    stepped = send(
        &def,
        stepped,
        Event::ResourceBusy {
            step: "build".to_string(),
            resource: "unity-editor".to_string(),
            at: AT.to_string(),
        },
    );

    assert_eq!(stepped.0.status, FlowStatus::WaitingExternal);
    assert_eq!(
        step_state(&stepped, "build").reason.as_deref(),
        Some("resource:unity-editor")
    );
    stepped = send(
        &def,
        stepped,
        Event::Retry {
            step: "build".to_string(),
            at: AT.to_string(),
        },
    );
    assert!(has_effect(&stepped, "build:1:start_pipeline"));
    assert_eq!(stepped.0.status, FlowStatus::Running);
}

#[test]
fn resource_busy_is_refused_outside_a_starting_pipeline() {
    let def = def_with(vec![action("a", NOOP)]);
    let stepped = start(&def);

    let result = advance(
        &def,
        stepped.0,
        Event::ResourceBusy {
            step: "a".to_string(),
            resource: "unity-editor".to_string(),
            at: AT.to_string(),
        },
    );

    assert!(result.is_err());
}

fn ceiling_hit(def: &FlowDef) -> Stepped {
    send(
        def,
        start(def),
        Event::TicketBudgetExceeded {
            step: "build".to_string(),
            detail: "Plafond atteint".to_string(),
            at: AT.to_string(),
        },
    )
}

#[test]
fn ticket_ceiling_makes_the_pipeline_step_wait_for_a_human() {
    let def = def_with(vec![pipeline("build")]);

    let stepped = ceiling_hit(&def);

    let held = step_state(&stepped, "build");
    assert_eq!(held.status, StepStatus::WaitingHuman);
    assert_eq!(
        held.reason.as_deref(),
        Some(StepReason::TicketBudget.as_str())
    );
    assert_eq!(held.detail.as_deref(), Some("Plafond atteint"));
}

#[test]
fn approving_the_ticket_ceiling_starts_the_pipeline_again() {
    let def = def_with(vec![pipeline("build")]);
    let stepped = ceiling_hit(&def);

    let stepped = send(&def, stepped, decide("build", "approve"));

    assert_eq!(stepped.0.status, FlowStatus::Running);
    assert_eq!(step_state(&stepped, "build").status, StepStatus::Running);
    assert!(has_effect(&stepped, "build:1:start_pipeline"));
}

#[test]
fn rejecting_the_ticket_ceiling_escalates_the_run_with_reason_ticket_budget() {
    let def = def_with(vec![pipeline("build")]);
    let stepped = ceiling_hit(&def);

    let stepped = send(&def, stepped, decide("build", "reject"));

    assert_eq!(stepped.0.status, FlowStatus::Escalated);
    assert_eq!(
        step_state(&stepped, "build").reason.as_deref(),
        Some(StepReason::TicketBudget.as_str())
    );
}

#[test]
fn ticket_ceiling_on_a_step_that_is_not_starting_is_rejected() {
    let def = def_with(vec![pipeline("build")]);
    let stepped = start(&def);
    let stepped = send(&def, stepped, pipeline_done("build", 0));

    let result = advance(
        &def,
        stepped.0,
        Event::TicketBudgetExceeded {
            step: "build".to_string(),
            detail: String::new(),
            at: AT.to_string(),
        },
    );

    assert!(result.is_err());
}

fn start_at(def: &FlowDef, entry: &str) -> Result<Stepped, FlowError> {
    let event = Event::Start {
        ticket: json!({"source": SOURCE_JIRA, "key": KEY}),
        at: AT.to_string(),
        entry: Some(entry.to_string()),
    };
    advance(def, FlowState::new(KEY, def.clone()), event)
}

#[test]
fn entry_start_runs_kept_steps_then_lands_with_assumed_outputs() {
    let def = def_of(ENTRY_MERGE);
    let stepped = start_at(&def, ENTRY_MERGE_NAME).unwrap();
    assert_eq!(stepped.0.entry.as_deref(), Some(ENTRY_MERGE_NAME));
    assert_eq!(stepped.0.current.as_deref(), Some(STEP_CHECKOUT));

    let stepped = send(&def, stepped, finish(STEP_CHECKOUT, 0));
    let skipped = step_state(&stepped, STEP_TEST_LOOP);
    assert_eq!(skipped.status, StepStatus::Skipped);
    assert_eq!(skipped.reason.as_deref(), Some(StepReason::Forced.as_str()));
    assert_eq!(
        step_state(&stepped, STEP_TEST).outputs[OUTPUT_PASSED],
        json!(true)
    );
    assert_eq!(stepped.0.current.as_deref(), Some(STEP_READY));

    let stepped = send(&def, stepped, finish(STEP_READY, 0));
    assert_eq!(stepped.0.current.as_deref(), Some(STEP_MERGE));
}

#[test]
fn entry_start_refuses_an_unknown_entry() {
    let def = def_of(ENTRY_MERGE);
    let reason = start_at(&def, ENTRY_UNKNOWN)
        .unwrap_err()
        .reject_reason()
        .expect("a rejected event");
    assert_eq!(reason, RejectReason::UnknownEntry);
}
