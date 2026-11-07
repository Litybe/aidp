use crate::exec_plan::compile_exec_plan;
use crate::exec_plan::test_support::{fixture_inputs, FixedPlan, NoPlan};
use crate::models::port::PlanSource;
use crate::pipeline;
use crate::run;
use jsonschema::{Draft, JSONSchema};

const SCHEMA_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../schema/execution-plan.schema.json"
);
fn compiled_schema() -> JSONSchema {
    let raw = std::fs::read_to_string(SCHEMA_PATH).expect("read execution plan schema");
    let schema: serde_json::Value = serde_json::from_str(&raw).expect("schema is JSON");
    JSONSchema::options()
        .with_draft(Draft::Draft7)
        .compile(&schema)
        .expect("schema compiles")
}

fn validation_errors(schema: &JSONSchema, value: &serde_json::Value) -> Vec<String> {
    match schema.validate(value) {
        Ok(()) => Vec::new(),
        Err(errors) => errors
            .map(|e| format!("{} at {}", e, e.instance_path))
            .collect(),
    }
}

#[test]
fn schema_parses_and_compiles() {
    compiled_schema();
}

#[test]
fn it_every_template_plan_validates_against_the_schema() {
    let schema = compiled_schema();
    let templates_dir = aidp_testkit::fixtures::templates_dir();
    let roots = [templates_dir.as_path()];
    let templates = pipeline::discover(&roots).unwrap();
    assert!(!templates.is_empty());
    let with_plan = FixedPlan::one_task_per_kind();
    let mut failures = Vec::new();
    for (id, path) in templates {
        let workspace = tempfile::tempdir().unwrap();
        let state = run::init(&path, workspace.path(), fixture_inputs(&id), None).unwrap();
        let pipe = pipeline::load(&state.pipeline_path).unwrap();
        let sources: [(&str, &dyn PlanSource); 2] =
            [("without plan", &NoPlan), ("with plan", &with_plan)];
        for (label, source) in sources {
            let plan = compile_exec_plan(&pipe, &state, source).unwrap();
            let value = serde_json::to_value(&plan).unwrap();
            for error in validation_errors(&schema, &value) {
                failures.push(format!("{id} ({label}): {error}"));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
