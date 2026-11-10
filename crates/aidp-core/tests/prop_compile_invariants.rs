//! Property-based tests for pipeline composition and expression evaluation invariants.
//!
//! These tests verify two core properties:
//! 1. `compose::expand` is idempotent on already-flat pipelines.
//! 2. `expr::eval_condition` is total (never panics) over a diverse space of inputs.
//!
//! By default, proptest runs with its built-in case count (typically 256 when PROPTEST_CASES
//! is unset). Set the `PROPTEST_CASES=256` environment variable at test time to ensure
//! this explicit count runs and is visible in the output.

use aidp_core::compose;
use aidp_core::expr::{self, EvalCtx};
use aidp_core::pipeline::{Pipeline, Stage};
use aidp_core::run::RunState;
use proptest::prelude::*;
use serde_json::json;
use std::collections::BTreeMap;
use std::path::PathBuf;

// ============================================================================
// Property 1: compose::expand idempotence
// ============================================================================

/// Strategy to generate valid stage names: `[a-z][a-z0-9_]{0,11}`.
fn stage_name_strategy() -> impl Strategy<Value = String> {
    prop::string::string_regex("[a-z][a-z0-9_]{0,11}")
        .expect("regex is valid")
        .prop_map(|name| name.to_string())
}

/// Create a stage with a given name and default fields.
fn make_stage(name: String) -> Stage {
    Stage {
        name,
        description: String::new(),
        policy: Vec::new(),
        r#type: None,
        uses: None,
        composite_entry: None,
        composite_when: vec![],
        agent: Some("executor".to_string()),
        model: None,
        effort: None,
        provider: None,
        skills: Vec::new(),
        skills_by: None,
        needs: Vec::new(),
        inputs: BTreeMap::new(),
        outputs: Vec::new(),
        with: BTreeMap::new(),
        when: None,
        gate: None,
        scope: None,
        isolation: None,
        tools: None,
        fan_out: None,
        merge_findings: None,
        on_harness_gap: None,
        plan_tasks: None,
        max_iterations: None,
        on_exhausted: None,
        knowledge: None,
        body: Vec::new(),
    }
}

/// Strategy to generate flat pipelines with 1..=5 distinct stages.
fn flat_pipeline_strategy() -> impl Strategy<Value = Pipeline> {
    // Generate 1..=5 distinct stage names.
    prop::collection::hash_set(stage_name_strategy(), 1..=5).prop_map(|names| {
        // For each set of names, create one Stage per name with default fields.
        let stages: Vec<Stage> = names.into_iter().map(make_stage).collect();

        Pipeline {
            version: 1,
            id: "flat_pipeline".to_string(),
            name: String::new(),
            description: String::new(),
            kind: "pipeline".to_string(),
            policy: Vec::new(),
            inputs: BTreeMap::new(),
            settings: Default::default(),
            stages,
        }
    })
}

proptest! {
    #[test]
    #[cfg_attr(miri, ignore)]
    fn prop_expand_idempotence(pipe in flat_pipeline_strategy()) {
        // expand(&p, &[]) should be a no-op on an already-flat pipeline.
        // We compare via serde_yaml::to_string because Pipeline and Stage do not
        // derive PartialEq — serialization equality is the only reliable comparison.
        let expanded = compose::expand(&pipe, &[])
            .expect("expansion of a flat pipeline must succeed");

        let original_yaml = serde_yaml::to_string(&pipe)
            .expect("original pipeline serializes");
        let expanded_yaml = serde_yaml::to_string(&expanded)
            .expect("expanded pipeline serializes");

        prop_assert_eq!(
            expanded_yaml, original_yaml,
            "compose::expand must be a no-op on a flat pipeline"
        );
    }

    #[test]
    #[cfg_attr(miri, ignore)]
    fn prop_eval_condition_never_panics(expr in condition_expr_strategy()) {
        let (state, pipe) = fixture_eval_ctx();
        let ctx = EvalCtx::new(&state, &pipe);

        // The property: expr::eval_condition must return (Some/None) without panicking,
        // even on malformed input. We do not assert a specific result, only that the
        // call is total (does not panic). This is checked through the public wrapper
        // because eval_bool_expr is private.
        let _result = expr::eval_condition(&expr, &ctx);
        // If we reach here, the function returned without panicking — success.
    }
}

// ============================================================================
// Property 2: expr::eval_condition totality
// ============================================================================

/// Strategy to generate well-formed and malformed expression strings.
fn condition_expr_strategy() -> impl Strategy<Value = serde_yaml::Value> {
    let well_formed = prop_oneof![
        // Bare literals
        Just("true"),
        Just("false"),
        Just("0"),
        Just("1"),
        Just("42"),
        Just("\"hello\""),
        Just("'world'"),
        // References (may not resolve, but should parse)
        Just("inputs.x"),
        Just("settings.max_correction_iterations"),
        Just("run_id"),
        // Equality comparisons
        Just("inputs.x == \"value\""),
        Just("inputs.x != \"value\""),
        Just("1 == 1"),
        Just("1 != 2"),
        // Logical operators
        Just("true && false"),
        Just("true || false"),
        Just("true && true || false"),
        // Numeric comparisons
        Just("1 > 0"),
        Just("1 >= 1"),
        Just("1 < 2"),
        Just("1 <= 1"),
        // Membership
        Just("inputs.x in [\"a\", \"b\"]"),
        // Filters
        Just("inputs.x | default(\"default\")"),
        // Nested conditions
        Just("(true)"),
        Just("{{ true }}"),
        Just("{{ inputs.x == \"y\" }}"),
    ];

    let malformed = prop_oneof![
        // Unbalanced quotes
        Just("\"unclosed"),
        Just("'unclosed"),
        Just("\"mixed'"),
        // Trailing operators
        Just("true &&"),
        Just("inputs.x ||"),
        Just("1 >"),
        // Empty strings
        Just(""),
        // Unbalanced braces
        Just("{{ unclosed"),
        Just("}} unopened"),
        // Nonsensical syntax
        Just("@#$%"),
        Just("   "),
    ];

    prop_oneof![well_formed, malformed].prop_map(|s| serde_yaml::Value::String(s.to_string()))
}

/// Create a minimal fixture pipeline and RunState for EvalCtx.
fn fixture_eval_ctx() -> (RunState, Pipeline) {
    let mut inputs = BTreeMap::new();
    inputs.insert("x".to_string(), json!("value"));

    let state = RunState {
        run_id: "test-run".to_string(),
        pipeline_path: PathBuf::from("/test.yaml"),
        pipeline_id: "test".to_string(),
        workspace_root: PathBuf::from("/ws"),
        run_dir: PathBuf::from("/ws/.aidp/runs/test"),
        started_at: "2024-01-01T00:00:00Z".to_string(),
        inputs,
        ..Default::default()
    };

    let pipe = Pipeline {
        version: 1,
        id: "test".to_string(),
        name: String::new(),
        description: String::new(),
        kind: "pipeline".to_string(),
        policy: Vec::new(),
        inputs: BTreeMap::new(),
        settings: Default::default(),
        stages: vec![make_stage("test".to_string())],
    };

    (state, pipe)
}
