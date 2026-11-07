use super::*;
use crate::exec_plan::test_support::{fixture_inputs, FixedPlan, NoPlan};
use crate::models::enums::AgentRole;
use crate::models::pipeline::KnowledgeLayer;
use crate::models::port::PlanSource;
use crate::pipeline;
use crate::run::{self, RunState};
use crate::skills;
use crate::workflow_emit;

const PARALLEL_LOG_PREFIX: &str = "log(\"parallel: ";
const FORBIDDEN_JS_TOKENS: [&str; 4] = ["await ", "await(", "agent(", "=>"];
const REVIEW_SIGNAL_SUFFIX: &str = ".outputs.findings.has_blocking_findings";
const MAX_WORKERS: usize = 4;
const NO_OVERRIDES: &[(&str, &str)] = &[];
const PLAN_STAGE: &str = "implement";
const REVIEW_BLOCKING_SIGNAL: &str = "stages.review__review.outputs.findings.has_blocking_findings";
const NON_TEXT_PANIC_MESSAGE: &str = "non-text panic";

struct Compiled {
    _workspace: tempfile::TempDir,
    state: RunState,
    pipe: pipeline::Pipeline,
}

fn init_template(id: &str, overrides: &[(&str, &str)]) -> Compiled {
    let path = aidp_testkit::fixtures::template(id);
    let workspace = tempfile::tempdir().unwrap();
    let mut inputs = fixture_inputs(id);
    if let Some(base) = inputs.as_object_mut() {
        for (key, value) in overrides {
            base.insert((*key).to_string(), (*value).into());
        }
    }
    let state = run::init(&path, workspace.path(), inputs, None).unwrap();
    let pipe = pipeline::load(&state.pipeline_path).unwrap();
    Compiled {
        _workspace: workspace,
        state,
        pipe,
    }
}

fn cross_every_composite(compiled: &mut Compiled) {
    let mut names: Vec<String> = pipeline::flat_stages(&compiled.pipe)
        .iter()
        .filter_map(|s| s.composite_entry.clone())
        .collect();
    names.sort();
    names.dedup();
    compiled.state.crossed_boundaries = names;
}

fn exec_plan_of(compiled: &Compiled, plans: &dyn PlanSource) -> ExecPlan {
    compile_exec_plan(&compiled.pipe, &compiled.state, plans).unwrap()
}

fn all_specs(steps: &[Step]) -> Vec<&StageSpec> {
    let mut specs = Vec::new();
    for step in steps {
        match step {
            Step::Stage(spec) => specs.push(spec),
            Step::Parallel { stages } => specs.extend(stages.iter()),
            Step::Loop { body, .. } => specs.extend(all_specs(body)),
            Step::Fanout { stage, .. } | Step::PlanTasks { stage, .. } => specs.push(stage),
            Step::CompositeBoundary { .. } | Step::Skipped { .. } => {}
        }
    }
    specs
}

fn parallel_groups(plan: &ExecPlan) -> Vec<String> {
    plan.steps
        .iter()
        .filter_map(|step| match step {
            Step::Parallel { stages: group } => Some(
                group
                    .iter()
                    .map(|s| s.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
            ),
            _ => None,
        })
        .collect()
}

fn logged_parallel_groups(script: &str) -> Vec<String> {
    script
        .lines()
        .filter_map(|line| line.trim().strip_prefix(PARALLEL_LOG_PREFIX))
        .map(|rest| rest.trim_end_matches("\");").to_string())
        .collect()
}

fn every_template_id() -> Vec<String> {
    let templates_dir = aidp_testkit::fixtures::templates_dir();
    let roots = [templates_dir.as_path()];
    let mut ids: Vec<String> = pipeline::discover(&roots)
        .unwrap()
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    ids.sort();
    ids
}

#[test]
fn dev_aidp_quick_track_shows_skipped_steps_with_reasons() {
    let mut compiled = init_template("dev-aidp", &[("track", "quick")]);
    cross_every_composite(&mut compiled);
    let plan = exec_plan_of(&compiled, &NoPlan);
    let skipped: Vec<(&str, SkipReason)> = plan
        .steps
        .iter()
        .filter_map(|step| match step {
            Step::Skipped { stage, reason } => Some((stage.as_str(), *reason)),
            _ => None,
        })
        .collect();
    assert!(
        skipped.contains(&("study", SkipReason::StaticWhen)),
        "got {skipped:?}"
    );
}

#[test]
fn a_review_loop_reads_the_blocking_findings_signal() {
    let mut compiled = init_template("dev-aidp", NO_OVERRIDES);
    cross_every_composite(&mut compiled);
    let plan = exec_plan_of(&compiled, &FixedPlan::one_task());
    let found = plan.steps.iter().find_map(|step| match step {
        Step::Loop {
            condition,
            signal,
            source_stage,
            body_producer,
            max,
            body,
            ..
        } => Some((
            condition,
            signal,
            source_stage,
            body_producer,
            *max,
            body.len(),
        )),
        _ => None,
    });
    let (condition, signal, source, producer, max, body_len) = found.expect("a loop step");
    assert!(condition
        .as_deref()
        .unwrap()
        .ends_with(REVIEW_SIGNAL_SUFFIX));
    assert_eq!(signal.as_deref(), Some("has_blocking_findings"));
    assert!(source.as_deref().unwrap().starts_with("review"));
    assert!(producer.is_some());
    assert!(max >= 1);
    assert!(body_len >= 1);
}

fn assert_parity(id: &str, plans: &dyn PlanSource) {
    let mut compiled = init_template(id, NO_OVERRIDES);
    cross_every_composite(&mut compiled);
    let emitted = workflow_emit::compile_with_source(&compiled.pipe, &compiled.state, plans);
    let (available_pipe, skill_warnings) = skills::without_unavailable(&compiled.pipe, false);
    let compiled_plan = compile_exec_plan(&available_pipe, &compiled.state, plans);
    let (Ok(emitted), Ok(plan)) = (emitted, compiled_plan) else {
        let again = workflow_emit::compile_with_source(&compiled.pipe, &compiled.state, plans);
        let plan_again = compile_exec_plan(&available_pipe, &compiled.state, plans);
        assert_eq!(again.is_err(), plan_again.is_err(), "error parity of {id}");
        return;
    };

    assert_eq!(plan.gates, emitted.gates, "gates of {id}");
    let plan_warnings: Vec<String> = plan
        .warnings
        .iter()
        .cloned()
        .chain(skill_warnings)
        .collect();
    assert_eq!(plan_warnings, emitted.warnings, "warnings of {id}");
    assert_eq!(
        parallel_groups(&plan),
        logged_parallel_groups(&emitted.script),
        "parallel groups of {id}"
    );
    assert_eq!(
        matches!(plan.end, Some(PassEnd::AwaitingPlan { .. })),
        emitted.awaiting_plan.is_some(),
        "awaiting plan of {id}"
    );
    assert_eq!(
        matches!(plan.end, Some(PassEnd::AwaitingComposite { .. })),
        emitted.awaiting_composite.is_some(),
        "awaiting composite of {id}"
    );
    assert_skips_and_soft_pauses(id, &plan, &emitted.script);
}

fn assert_skips_and_soft_pauses(id: &str, plan: &ExecPlan, script: &str) {
    let awaiting_stage = match &plan.end {
        Some(PassEnd::AwaitingPlan { stage, .. }) => Some(stage.as_str()),
        _ => None,
    };
    for step in &plan.steps {
        if let Step::Skipped { stage, .. } = step {
            let line = format!("skip `{stage}` (when=false at compile time)");
            assert!(script.contains(&line), "{id}: missing `{line}`");
        }
    }
    for spec in all_specs(&plan.steps) {
        if awaiting_stage == Some(spec.name.as_str()) {
            continue;
        }
        let check = format!("isPause(results[\"{}\"], {})", spec.name, spec.soft_pause);
        assert!(script.contains(&check), "{id}: missing `{check}`");
    }
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| {
            payload
                .downcast_ref::<&str>()
                .map(|text| (*text).to_string())
        })
        .unwrap_or_else(|| NON_TEXT_PANIC_MESSAGE.to_string())
}

/// Checks parity of every template on a bounded thread pool, then panics once listing all failures.
fn assert_parity_for_every_template(make_plans: fn() -> Box<dyn PlanSource>) {
    let ids = every_template_id();
    let available = std::thread::available_parallelism().map_or(1, |count| count.get());
    let worker_count = MAX_WORKERS.min(available).min(ids.len()).max(1);
    let next_index = std::sync::atomic::AtomicUsize::new(0);
    let failures = std::sync::Mutex::new(Vec::<String>::new());
    std::thread::scope(|scope| {
        for _ in 0..worker_count {
            scope.spawn(|| loop {
                let index = next_index.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let Some(id) = ids.get(index) else { break };
                let plans = make_plans();
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    assert_parity(id, plans.as_ref())
                }));
                if let Err(payload) = outcome {
                    failures
                        .lock()
                        .unwrap()
                        .push(format!("{id}: {}", panic_message(payload.as_ref())));
                }
            });
        }
    });
    let mut failures = failures.into_inner().unwrap();
    failures.sort();
    assert!(
        failures.is_empty(),
        "parity failures:\n{}",
        failures.join("\n")
    );
}

#[test]
fn it_compiled_plan_matches_the_emitter_before_a_plan_exists() {
    assert_parity_for_every_template(|| Box::new(NoPlan));
}

#[test]
fn it_compiled_plan_matches_the_emitter_once_a_plan_exists() {
    assert_parity_for_every_template(|| Box::new(FixedPlan::one_task()));
}

#[test]
fn dev_csharp_has_parallel_groups_a_loop_and_soft_reviewers() {
    let mut compiled = init_template("dev-csharp", NO_OVERRIDES);
    cross_every_composite(&mut compiled);
    let plan = exec_plan_of(&compiled, &FixedPlan::one_task());
    assert!(!parallel_groups(&plan).is_empty());
    assert!(plan.steps.iter().any(|s| matches!(s, Step::Loop { .. })));
    let specs = all_specs(&plan.steps);
    assert!(specs
        .iter()
        .filter(|s| s.agent == AgentRole::Reviewer)
        .all(|s| s.soft_pause));
    assert!(specs
        .iter()
        .any(|s| s.contracts.contains(&ContractFlag::ReviewSynthesis)));
}

#[test]
fn gha_fix_and_review_compile_gates_and_parallel_groups() {
    let gha = exec_plan_of(&init_template("gha-fix", NO_OVERRIDES), &NoPlan);
    assert!(gha
        .steps
        .iter()
        .any(|s| matches!(s, Step::PlanTasks { .. })));
    let mut review = init_template("review", NO_OVERRIDES);
    cross_every_composite(&mut review);
    let review_plan = exec_plan_of(&review, &NoPlan);
    assert!(!parallel_groups(&review_plan).is_empty());
}

#[test]
fn a_small_pass_budget_stops_before_a_composite() {
    let mut compiled = init_template("dev-composed", NO_OVERRIDES);
    compiled.pipe.settings.max_stages_per_pass = Some(1);
    let plan = exec_plan_of(&compiled, &FixedPlan::one_task());
    let Some(PassEnd::AwaitingComposite { composite, stage }) = &plan.end else {
        panic!("expected a composite boundary, got {:?}", plan.end);
    };
    assert!(matches!(
        plan.steps.last(),
        Some(Step::CompositeBoundary { composite: c, stage: s }) if c == composite && s == stage
    ));
    cross_every_composite(&mut compiled);
    compiled.state.crossed_boundaries.push(composite.clone());
    let crossed = exec_plan_of(&compiled, &FixedPlan::one_task());
    assert!(!matches!(
        crossed.end,
        Some(PassEnd::AwaitingComposite { .. })
    ));
}

#[test]
fn a_missing_plan_ends_the_pass_and_a_present_plan_continues_it() {
    let mut compiled = init_template("dev-aidp", NO_OVERRIDES);
    cross_every_composite(&mut compiled);
    let waiting = exec_plan_of(&compiled, &NoPlan);
    assert!(matches!(
        &waiting.end,
        Some(PassEnd::AwaitingPlan { stage, .. }) if stage == PLAN_STAGE
    ));

    let ready = exec_plan_of(&compiled, &FixedPlan::one_task());
    assert!(!matches!(ready.end, Some(PassEnd::AwaitingPlan { .. })));
    assert!(ready
        .steps
        .iter()
        .any(|s| matches!(s, Step::PlanTasks { stage, .. } if stage.name == PLAN_STAGE)));
}

#[test]
fn gates_are_flagged_with_their_approval_state() {
    let mut compiled = init_template("dev-csharp", NO_OVERRIDES);
    cross_every_composite(&mut compiled);
    let plan = exec_plan_of(&compiled, &FixedPlan::one_task());
    assert!(!plan.gates.is_empty());
    for spec in all_specs(&plan.steps) {
        assert_eq!(
            spec.gate.is_some(),
            plan.gates.contains(&spec.name),
            "gate flag of {}",
            spec.name
        );
    }
}

#[test]
fn knowledge_stages_name_their_layer() {
    let mut compiled = init_template("knowledge-init", NO_OVERRIDES);
    cross_every_composite(&mut compiled);
    let plan = exec_plan_of(&compiled, &NoPlan);
    assert!(all_specs(&plan.steps).iter().any(|s| s
        .contracts
        .contains(&ContractFlag::Knowledge(KnowledgeLayer::Feature))));
}

#[test]
fn it_a_serialized_plan_contains_no_javascript() {
    for id in every_template_id() {
        let mut compiled = init_template(&id, NO_OVERRIDES);
        cross_every_composite(&mut compiled);
        for plans in [&NoPlan as &dyn PlanSource, &FixedPlan::one_task()] {
            let Ok(plan) = compile_exec_plan(&compiled.pipe, &compiled.state, plans) else {
                continue;
            };
            let json = serde_json::to_string(&plan).unwrap();
            for token in FORBIDDEN_JS_TOKENS {
                assert!(!json.contains(token), "{id}: `{token}` found in {json}");
            }
        }
    }
}

/// The `dev` correction loop reads the blocking-findings signal of the review stage.
#[test]
fn dev_correction_loop_reads_the_blocking_findings_signal() {
    let mut compiled = init_template("dev", NO_OVERRIDES);
    cross_every_composite(&mut compiled);
    let plan = exec_plan_of(&compiled, &FixedPlan::one_task());
    let conditions: Vec<&str> = plan
        .steps
        .iter()
        .filter_map(|step| match step {
            Step::Loop { condition, .. } => condition.as_deref(),
            _ => None,
        })
        .collect();
    assert_eq!(conditions, [REVIEW_BLOCKING_SIGNAL]);
}

/// `pr-comments` has no review stage, so it compiles without any correction loop.
#[test]
fn pr_comments_compiles_without_a_correction_loop() {
    let compiled = init_template("pr-comments", NO_OVERRIDES);
    let plan = exec_plan_of(&compiled, &FixedPlan::one_task());
    assert!(!plan.steps.iter().any(|s| matches!(s, Step::Loop { .. })));
}
