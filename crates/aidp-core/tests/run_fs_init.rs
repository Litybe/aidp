use aidp_core::models::constants::messages::UNKNOWN_REFUSAL;
use aidp_core::models::constants::stack as stack_const;
use aidp_core::models::enums::Stack;
use aidp_core::models::stack_inputs::StackInputs;
use aidp_core::pipeline::Pipeline;
use aidp_core::run::*;

use crate::common;
use common::*;

#[test]
fn init_populates_plan_layers() {
    let ws = scratch_dir();
    std::fs::create_dir_all(&ws).unwrap();
    let p = ws.join("pipe.yaml");
    std::fs::write(
        &p,
        r#"
version: 5
id: planish
stages:
  - name: discover
    agent: analyzer
    outputs: [context.md]
  - name: code
    agent: executor
    needs: [discover]
    outputs: [changed-files.json]
  - name: review
    agent: reviewer
    needs: [code]
    outputs: [findings.json]
"#,
    )
    .unwrap();
    let st = init(&p, &ws, serde_json::json!({}), None).unwrap();
    assert_eq!(
        st.plan,
        vec![
            vec!["discover".to_string()],
            vec!["code".to_string()],
            vec!["review".to_string()],
        ],
        "init must populate topo layers"
    );
}

const CYCLIC_PIPE: &str = include_str!("fixtures/cycle/cyclic-pipeline.yaml");
const CYCLE_ERROR_FRAGMENT: &str = "cycle or unresolved dep";
const CYCLE_STAGE_A: &str = "a";
const CYCLE_STAGE_B: &str = "b";

#[test]
fn init_refuses_a_cyclic_pipeline_naming_both_stages() {
    let ws = scratch_dir();
    std::fs::create_dir_all(&ws).unwrap();
    let p = ws.join("pipe.yaml");
    std::fs::write(&p, CYCLIC_PIPE).unwrap();

    let Err(error) = init(&p, &ws, serde_json::json!({}), None) else {
        panic!("init must refuse a cyclic stage graph");
    };
    let message = error.to_string();

    assert!(message.contains(CYCLE_ERROR_FRAGMENT), "{message}");
    assert!(
        message.contains(&format!("\"{CYCLE_STAGE_A}\"")),
        "{message}"
    );
    assert!(
        message.contains(&format!("\"{CYCLE_STAGE_B}\"")),
        "{message}"
    );
}

#[test]
fn init_refuses_a_cyclic_pipeline_leaving_the_vault_untouched() {
    let (_, ws, vault) = vault_fixture();
    let p = ws.join("pipe.yaml");
    std::fs::write(&p, CYCLIC_PIPE).unwrap();

    let Err(error) = init(&p, &ws, serde_json::json!({}), Some(&vault)) else {
        panic!("init must refuse a cyclic stage graph even with a vault");
    };
    let message = error.to_string();
    assert!(message.contains(CYCLE_ERROR_FRAGMENT), "{message}");

    assert!(
        !vault.pipelines_dir().exists(),
        "no vault skeleton may be created for a refused run"
    );
    assert!(
        !vault.knowledge_root().exists(),
        "no vault skeleton may be created for a refused run"
    );
}

const AUTO_STACK_PIPE: &str = include_str!("fixtures/auto-stack/pipeline.yaml");
const AUTO_STACK_NARROW_PIPE: &str = include_str!("fixtures/auto-stack/narrow.yaml");
const UNKNOWN_STACK_PIPE: &str = include_str!("fixtures/stack-unknown/pipeline.yaml");
const REQUIRED_STACK_PIPE: &str = include_str!("fixtures/required-stack/pipeline.yaml");
const AUTO_STACK_PIPE_ID: &str = "autostack";
const PIPE_FILE: &str = "pipe.yaml";
const CARGO_TOML: &str = "Cargo.toml";
const FIX_STAGE: &str = "fix";
const BASE_SKILL: &str = "a";
const RUST_BUCKET_SKILL: &str = "r";
const DEFAULT_BUCKET_SKILL: &str = "g";

/// Runs `init` on `pipe` in a fresh workspace holding `marker`, if any.
fn init_auto_stack(
    pipe: &str,
    marker: Option<&str>,
    inputs: serde_json::Value,
) -> anyhow::Result<RunState> {
    let ws = scratch_dir();
    std::fs::create_dir_all(&ws).unwrap();
    if let Some(m) = marker {
        std::fs::write(ws.join(m), "").unwrap();
    }
    let pipe_path = ws.join(PIPE_FILE);
    std::fs::write(&pipe_path, pipe).unwrap();
    init(&pipe_path, &ws, inputs, None)
}

/// The resolved skills of the fixture's only stage.
fn fix_skills(st: &RunState) -> Vec<String> {
    let eff: Pipeline =
        serde_yaml::from_str(&std::fs::read_to_string(&st.pipeline_path).unwrap()).unwrap();
    eff.stages
        .iter()
        .find(|s| s.name == FIX_STAGE)
        .unwrap()
        .skills
        .clone()
}

/// A declared stack input resolved to `unknown` fails init with the refusal.
#[test]
fn init_refuses_an_unknown_stack() {
    let inputs = serde_json::to_value(StackInputs {
        stack: Stack::Unknown,
    })
    .unwrap();
    let error = init_auto_stack(UNKNOWN_STACK_PIPE, None, inputs)
        .unwrap_err()
        .to_string();
    assert_eq!(error, UNKNOWN_REFUSAL);
}

/// `stack: auto` becomes the detected stack before `skills_by` reads it; with
/// no marker it becomes `generic`, whose bucket is the stage's default list.
#[test]
fn init_replaces_auto_stack_with_the_detected_one() {
    let st = init_auto_stack(AUTO_STACK_PIPE, Some(CARGO_TOML), serde_json::json!({})).unwrap();
    assert_eq!(st.inputs[stack_const::INPUT], stack_const::RUST);
    assert_eq!(fix_skills(&st), vec![BASE_SKILL, RUST_BUCKET_SKILL]);

    let st = init_auto_stack(AUTO_STACK_PIPE, None, serde_json::json!({})).unwrap();
    assert_eq!(st.inputs[stack_const::INPUT], stack_const::GENERIC);
    assert_eq!(fix_skills(&st), vec![BASE_SKILL, DEFAULT_BUCKET_SKILL]);
}

/// A required stack without default, left out, becomes the project stack of the workspace.
#[test]
fn init_fills_a_missing_required_stack_with_the_project_stack() {
    let st = init_auto_stack(REQUIRED_STACK_PIPE, Some(CARGO_TOML), serde_json::json!({})).unwrap();
    assert_eq!(st.inputs[stack_const::INPUT], stack_const::RUST);
}

/// A missing required stack on a workspace with no marker is refused as unknown, not as missing.
#[test]
fn init_refuses_a_missing_required_stack_it_cannot_detect() {
    let error = init_auto_stack(REQUIRED_STACK_PIPE, None, serde_json::json!({}))
        .unwrap_err()
        .to_string();
    assert_eq!(error, UNKNOWN_REFUSAL);
}

/// An explicit stack wins over what the workspace markers say.
#[test]
fn init_keeps_an_explicit_stack_over_detection() {
    let st = init_auto_stack(
        AUTO_STACK_PIPE,
        Some(CARGO_TOML),
        serde_json::json!({ (stack_const::INPUT): stack_const::CSHARP }),
    )
    .unwrap();
    assert_eq!(st.inputs[stack_const::INPUT], stack_const::CSHARP);
    assert_eq!(fix_skills(&st), vec![BASE_SKILL, DEFAULT_BUCKET_SKILL]);
}

/// A detected stack the template's `values:` do not allow fails the init.
#[test]
fn init_rejects_a_detected_stack_the_template_does_not_allow() {
    let err = init_auto_stack(
        AUTO_STACK_NARROW_PIPE,
        Some(CARGO_TOML),
        serde_json::json!({}),
    )
    .unwrap_err()
    .to_string();
    assert!(
        err.contains(&format!("detected {:?}", stack_const::RUST))
            && err.contains(AUTO_STACK_PIPE_ID),
        "{err}"
    );
}

const CI_FIX_TEMPLATES: &[&str] = &["gha-fix", "sonar-fix", "pr-comments"];
const LANGUAGE_STAGES: &[&str] = &["fix"];
const OPTIONAL_LANGUAGE_STAGES: &[&str] = &["refix", "tests"];
const FOREIGN_SKILLS: &[&str] = &["csharp-dev-aidp", "lambda-ts-dev-aidp"];
const RUST_DEV_SKILL: &str = "rust-dev-aidp";

/// The shipped CI-fix templates, initialised on a Rust workspace, give their
/// code/tests stages the Rust skills and neither the C# nor the Lambda TS one.
#[test]
fn ci_fix_templates_on_a_rust_repo_load_rust_skills_only() {
    for id in CI_FIX_TEMPLATES {
        let tpl = aidp_testkit::fixtures::template(id);
        let ws = scratch_dir();
        std::fs::create_dir_all(ws.join("rust")).unwrap();
        std::fs::write(ws.join("rust").join(CARGO_TOML), "").unwrap();

        let st = init(&tpl, &ws, serde_json::json!({}), None).unwrap();
        let eff: Pipeline =
            serde_yaml::from_str(&std::fs::read_to_string(&st.pipeline_path).unwrap()).unwrap();

        assert_eq!(st.inputs[stack_const::INPUT], stack_const::RUST, "{id}");
        let all: Vec<(&str, &Vec<String>)> = aidp_core::pipeline::flat_stages(&eff)
            .into_iter()
            .map(|s| (s.name.as_str(), &s.skills))
            .collect();
        for (stage, skills) in &all {
            assert!(
                !skills.iter().any(|s| FOREIGN_SKILLS.contains(&s.as_str())),
                "{id}/{stage} still declares a C#/TS skill: {skills:?}"
            );
        }
        for stage in LANGUAGE_STAGES {
            assert!(
                all.iter().any(|(n, _)| n == stage),
                "{id} has no `{stage}` stage"
            );
        }
        for (stage, skills) in all
            .iter()
            .filter(|(n, _)| LANGUAGE_STAGES.contains(n) || OPTIONAL_LANGUAGE_STAGES.contains(n))
        {
            assert!(
                skills.iter().any(|s| s == RUST_DEV_SKILL),
                "{id}/{stage}: {skills:?}"
            );
        }
    }
}

const SKILLS_BY_PIPE: &str = r#"
version: 5
id: skillsby
inputs:
  stack:
    required: true
    kind: enum
    values: [generic, lambda-ts]
    default: generic
stages:
  - name: implement
    agent: executor
    skills: [a]
    skills_by:
      input: stack
      map:
        lambda-ts: [b, c]
      default: [d]
    outputs: [out.txt]
"#;

#[test]
fn init_resolves_skills_by_from_an_explicit_input() {
    let ws = scratch_dir();
    std::fs::create_dir_all(&ws).unwrap();
    let p = ws.join("pipe.yaml");
    std::fs::write(&p, SKILLS_BY_PIPE).unwrap();

    let st = init(&p, &ws, serde_json::json!({"stack": "lambda-ts"}), None).unwrap();

    let eff = std::fs::read_to_string(&st.pipeline_path).unwrap();
    assert!(
        !eff.contains("skills_by"),
        "effective-pipeline.yaml must not retain a resolved skills_by key: {eff}"
    );
    let effective: Pipeline = serde_yaml::from_str(&eff).unwrap();
    let stage = effective
        .stages
        .iter()
        .find(|s| s.name == "implement")
        .unwrap();
    assert_eq!(
        stage.skills,
        vec!["a".to_string(), "b".to_string(), "c".to_string()],
        "base [a] must be followed by the mapped lambda-ts skills [b, c]"
    );
}

#[test]
fn init_resolves_skills_by_from_the_declared_default_when_no_input_is_given() {
    let ws = scratch_dir();
    std::fs::create_dir_all(&ws).unwrap();
    let p = ws.join("pipe.yaml");
    std::fs::write(&p, SKILLS_BY_PIPE).unwrap();

    let st = init(&p, &ws, serde_json::json!({}), None).unwrap();

    let eff = std::fs::read_to_string(&st.pipeline_path).unwrap();
    assert!(!eff.contains("skills_by"), "eff must drop skills_by: {eff}");
    let effective: Pipeline = serde_yaml::from_str(&eff).unwrap();
    let stage = effective
        .stages
        .iter()
        .find(|s| s.name == "implement")
        .unwrap();
    assert_eq!(
        stage.skills,
        vec!["a".to_string(), "d".to_string()],
        "unmapped input value must fall back to SkillsBy's own default"
    );
}

#[test]
fn init_resolves_skills_by_inside_a_loop_body() {
    let ws = scratch_dir();
    std::fs::create_dir_all(&ws).unwrap();
    let p = ws.join("pipe.yaml");
    std::fs::write(
        &p,
        r#"
version: 5
id: skillsbyloop
inputs:
  stack:
    required: true
    kind: enum
    values: [generic, lambda-ts]
    default: generic
stages:
  - name: correction_loop
    type: loop
    when: true
    max_iterations: 1
    body:
      - name: refix
        agent: executor
        skills: [a]
        skills_by:
          input: stack
          map:
            lambda-ts: [b, c]
          default: [d]
        outputs: [out.txt]
"#,
    )
    .unwrap();

    let st = init(&p, &ws, serde_json::json!({"stack": "lambda-ts"}), None).unwrap();

    let eff = std::fs::read_to_string(&st.pipeline_path).unwrap();
    assert!(
        !eff.contains("skills_by"),
        "a loop body's skills_by must also be resolved and dropped: {eff}"
    );
    let effective: Pipeline = serde_yaml::from_str(&eff).unwrap();
    let loop_stage = effective
        .stages
        .iter()
        .find(|s| s.name == "correction_loop")
        .unwrap();
    let body_stage = loop_stage.body.iter().find(|s| s.name == "refix").unwrap();
    assert_eq!(
        body_stage.skills,
        vec!["a".to_string(), "b".to_string(), "c".to_string()],
        "a stage inside a loop body must resolve skills_by exactly like a top-level stage"
    );
}

#[test]
fn init_with_an_undeclared_input_records_a_warning() {
    let ws = scratch_dir();
    std::fs::create_dir_all(&ws).unwrap();
    let p = ws.join("pipe.yaml");
    std::fs::write(&p, GROUPED).unwrap();

    let st = init(
        &p,
        &ws,
        serde_json::json!({"target": "PIXEL-8", "not_a_declared_input": "x"}),
        None,
    )
    .unwrap();

    assert!(
        st.init_warnings
            .iter()
            .any(|w| w.contains("not_a_declared_input")),
        "init_warnings must name the undeclared input: {:?}",
        st.init_warnings
    );
}
