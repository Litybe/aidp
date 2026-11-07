//! Compiles the pipelines a ticket flow runs and checks their stack, branch, PR and publish contracts.

use aidp_core::clock::FixedClock;
use aidp_core::cost::PriceTable;
use aidp_core::git::RecordingGit;
use aidp_core::id::FixedIdGen;
use aidp_core::models::constants::messages::UNKNOWN_REFUSAL;
use aidp_core::models::enums::pr_mode::PrMode;
use aidp_core::models::enums::stack::Stack;
use aidp_core::pipeline;
use aidp_core::plan::{self, Plan};
use aidp_core::run::{init_with, record_resume_with, RunPorts};
use aidp_core::workflow_emit::{compile_with_source, PlanSource};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use tempfile::TempDir;

const PLAN_FIXTURE_PATH: &str = "tests/fixtures/workflow_emit_golden/plan.json";
const DEV_FLOW_ID: &str = "dev-flow";
const CORRECTION_ID: &str = "correction-after-test-dev";
const FIXED_RUN_ID: &str = "20260101-000000-flow";
const FIXED_TIME: &str = "2026-01-01T00:00:00+00:00";
const STACK_INPUT: &str = "stack";
const TRACK_INPUT: &str = "track";
const PR_INPUT: &str = "pr";
const BRANCH_READY_INPUT: &str = "branch_ready";
const PROMPT_INPUT: &str = "prompt";
const JIRA_KEYS_INPUT: &str = "jira_keys";
const FEATURE_INPUT: &str = "feature";
const STANDARD_TRACK: &str = "standard";
const FEATURE_LABEL: &str = "flow-feature";
const PROMPT_TEXT: &str = "add a health endpoint";
const JIRA_KEY: &str = "LOC-1";
const PREPARE_BRANCH_STAGE: &str = "prepare_branch";
const PUSH_PR_STAGE: &str = "push_pr";
const PUSH_COMMENT_STAGE: &str = "push_comment";
const DISCOVER_JIRA_STAGE: &str = "discover__discover_jira";
const DISCOVER_PROMPT_STAGE: &str = "discover__discover_prompt";
const DISCOVER_SYNTHESIS_STAGE: &str = "discover__discover";
const REQUIREMENTS_NOTE: &str = "context.md";
const PR_CREATE_MARKER: &str = "pr-create";
const PR_COMMENT_MARKER: &str = "pr-comment";
const DRAFT_FLAG: &str = "--draft";
const ASSUMPTIONS_HEADING: &str = "Hypothèses";
const TESTS_CSHARP_STAGE: &str = "tests_csharp";
const TESTS_LAMBDA_TS_STAGE: &str = "tests_lambda_ts";
const TESTS_NODE_STAGE: &str = "tests_node";
const TESTS_RUST_STAGE: &str = "tests_rust";
const COMPOSITE_SEPARATOR: &str = "__";
const SKIP_LOG: &str = "skip `";
const TICK: &str = "`";
const QUOTE: &str = "\"";
const MAX_COMPOSITE_PASSES: usize = 32;
const PUBLISH_INPUT: &str = "publish";
const PUBLISH_BY_HOST: &str = "host";
const PUBLISH_BY_AGENT: &str = "agent";
const GIT_PUSH_SKILL_CALL: &str = r#"Skill(\"git-push\")"#;
const PUBLISHING_TEMPLATES: &[&str] = &["gha-fix", "sonar-fix", "pr-comments", CORRECTION_ID];

/// Serves the same fixture plan for every `plan_tasks:` stage.
struct FixturePlanSource {
    plan: Plan,
}

impl PlanSource for FixturePlanSource {
    fn load(&self, _path: &Path) -> anyhow::Result<Option<Plan>> {
        Ok(Some(self.plan.clone()))
    }
}

fn template_path(relative: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(relative)
}

/// Compiles template `id` of the frozen skill over `inputs` in a scratch workspace and returns the emitted script.
fn compile_script(id: &str, inputs: Value) -> anyhow::Result<String> {
    let workspace = TempDir::new().expect("temp workspace");
    let clock = FixedClock::from_rfc3339(FIXED_TIME);
    let ids = FixedIdGen(FIXED_RUN_ID.to_string());
    let git = RecordingGit::new();
    let prices = PriceTable::from_env_or_embedded();
    let ports = RunPorts {
        clock: &clock,
        ids: &ids,
        git: &git,
        prices: &prices,
    };
    let mut state = init_with(
        &aidp_testkit::fixtures::template(id),
        workspace.path(),
        inputs,
        None,
        &ports,
    )?;
    let plans = FixturePlanSource {
        plan: plan::load(&template_path(PLAN_FIXTURE_PATH))?,
    };
    for _ in 0..MAX_COMPOSITE_PASSES {
        let pipe = pipeline::load(&state.pipeline_path)?;
        let emitted = compile_with_source(&pipe, &state, &plans)?;
        let Some(awaiting) = emitted.awaiting_composite else {
            return Ok(emitted.script);
        };
        record_resume_with(
            &mut state,
            None,
            &[],
            None,
            Some(awaiting.composite),
            &ports,
        )?;
    }
    anyhow::bail!("composites did not settle within {MAX_COMPOSITE_PASSES} passes")
}

fn flow_inputs(stack: Stack) -> serde_json::Map<String, Value> {
    let mut inputs = serde_json::Map::new();
    inputs.insert(STACK_INPUT.into(), serde_json::to_value(stack).unwrap());
    inputs.insert(TRACK_INPUT.into(), json!(STANDARD_TRACK));
    inputs.insert(PROMPT_INPUT.into(), json!(PROMPT_TEXT));
    inputs.insert(FEATURE_INPUT.into(), json!(FEATURE_LABEL));
    inputs
}

fn dev_flow(stack: Stack, overrides: &[(&str, Value)]) -> anyhow::Result<String> {
    let mut inputs = flow_inputs(stack);
    for (name, value) in overrides {
        inputs.insert((*name).into(), value.clone());
    }
    compile_script(DEV_FLOW_ID, Value::Object(inputs))
}

/// True when `stage` (or a composite stage under it) is dispatched and not logged as skipped.
fn runs_stage(script: &str, stage: &str) -> bool {
    let skipped = [
        format!("{SKIP_LOG}{stage}{TICK}"),
        format!("{SKIP_LOG}{stage}{COMPOSITE_SEPARATOR}"),
    ];
    let dispatched = [
        format!("{QUOTE}{stage}{QUOTE}"),
        format!("{QUOTE}{stage}{COMPOSITE_SEPARATOR}"),
    ];
    let any = |markers: &[String]| markers.iter().any(|marker| script.contains(marker));
    any(&dispatched) && !any(&skipped)
}

fn stack_markers(script: &str) -> [bool; 4] {
    [
        runs_stage(script, TESTS_CSHARP_STAGE),
        runs_stage(script, TESTS_LAMBDA_TS_STAGE),
        runs_stage(script, TESTS_NODE_STAGE),
        runs_stage(script, TESTS_RUST_STAGE),
    ]
}

#[test]
fn dev_flow_template_keeps_only_the_csharp_test_stages() {
    let script = dev_flow(Stack::Csharp, &[]).unwrap();
    assert_eq!(stack_markers(&script), [true, false, false, false]);
}

#[test]
fn dev_flow_template_keeps_only_the_lambda_ts_test_stages() {
    let script = dev_flow(Stack::LambdaTs, &[]).unwrap();
    assert_eq!(stack_markers(&script), [false, true, false, false]);
}

#[test]
fn dev_flow_template_keeps_only_the_rust_test_stages() {
    let script = dev_flow(Stack::Rust, &[]).unwrap();
    assert_eq!(stack_markers(&script), [false, false, false, true]);
}

#[test]
fn dev_flow_template_keeps_only_the_ts_node_test_stages() {
    let script = dev_flow(Stack::TsNode, &[]).unwrap();
    assert_eq!(stack_markers(&script), [false, false, true, false]);
}

#[test]
fn dev_flow_template_routes_angular_to_the_vitest_stages() {
    let script = dev_flow(Stack::Angular, &[]).unwrap();
    assert_eq!(stack_markers(&script), [false, false, true, false]);
}

#[test]
fn dev_flow_template_skips_branch_setup_when_the_branch_is_ready() {
    let script = dev_flow(Stack::Rust, &[(BRANCH_READY_INPUT, json!(true))]).unwrap();
    assert!(!runs_stage(&script, PREPARE_BRANCH_STAGE));
}

#[test]
fn dev_flow_template_runs_branch_setup_when_the_branch_is_not_ready() {
    let script = dev_flow(Stack::Rust, &[(BRANCH_READY_INPUT, json!(false))]).unwrap();
    assert!(runs_stage(&script, PREPARE_BRANCH_STAGE));
}

#[test]
fn dev_flow_template_opens_a_draft_pr_with_an_assumptions_section() {
    let script = dev_flow(
        Stack::Rust,
        &[(PR_INPUT, serde_json::to_value(PrMode::Draft).unwrap())],
    )
    .unwrap();
    assert!(runs_stage(&script, PUSH_PR_STAGE));
    assert!(script.contains(PR_CREATE_MARKER));
    assert!(script.contains(DRAFT_FLAG));
    assert!(script.contains(ASSUMPTIONS_HEADING));
}

#[test]
fn dev_flow_template_drops_push_pr_when_pr_is_none() {
    let script = dev_flow(
        Stack::Rust,
        &[(PR_INPUT, serde_json::to_value(PrMode::None).unwrap())],
    )
    .unwrap();
    assert!(!runs_stage(&script, PUSH_PR_STAGE));
}

#[test]
fn dev_flow_template_refuses_the_unknown_stack() {
    let error = dev_flow(Stack::Unknown, &[]).unwrap_err();
    assert!(format!("{error:#}").contains(UNKNOWN_REFUSAL));
}

#[test]
fn dev_flow_template_prompt_demand_keeps_the_requirements_note() {
    let script = dev_flow(Stack::Rust, &[]).unwrap();
    assert!(runs_stage(&script, DISCOVER_PROMPT_STAGE));
    assert!(runs_stage(&script, DISCOVER_SYNTHESIS_STAGE));
    assert!(script.contains(REQUIREMENTS_NOTE));
}

#[test]
fn dev_flow_template_jira_demand_keeps_the_requirements_note() {
    let overrides = [
        (JIRA_KEYS_INPUT, json!([JIRA_KEY])),
        (PROMPT_INPUT, json!("")),
    ];
    let script = dev_flow(Stack::Rust, &overrides).unwrap();
    assert!(runs_stage(&script, DISCOVER_JIRA_STAGE));
    assert!(script.contains(REQUIREMENTS_NOTE));
}

/// Compiles template `id` with `publish` set; the correction template also gets its stack and report.
fn publishing_script(id: &str, publish: &str) -> String {
    let mut inputs = if id == CORRECTION_ID {
        flow_inputs(Stack::Rust)
    } else {
        serde_json::Map::new()
    };
    inputs.insert(PUBLISH_INPUT.into(), json!(publish));
    compile_script(id, Value::Object(inputs)).unwrap()
}

#[test]
fn host_publishing_templates_never_load_the_git_push_skill() {
    for id in PUBLISHING_TEMPLATES {
        let script = publishing_script(id, PUBLISH_BY_HOST);
        assert!(!script.contains(GIT_PUSH_SKILL_CALL), "{id} loads git-push");
    }
}

#[test]
fn agent_publishing_templates_keep_the_git_push_skill() {
    for id in PUBLISHING_TEMPLATES {
        let script = publishing_script(id, PUBLISH_BY_AGENT);
        assert!(script.contains(GIT_PUSH_SKILL_CALL), "{id} lost git-push");
    }
}

#[test]
fn correction_template_comments_the_pr_and_never_creates_one() {
    let mut inputs = flow_inputs(Stack::Rust);
    inputs.remove(FEATURE_INPUT);
    let script = compile_script(CORRECTION_ID, Value::Object(inputs)).unwrap();
    assert!(!script.contains(PR_CREATE_MARKER));
    assert!(script.contains(PR_COMMENT_MARKER));
    assert!(runs_stage(&script, PUSH_COMMENT_STAGE));
    assert!(!runs_stage(&script, PREPARE_BRANCH_STAGE));
}
