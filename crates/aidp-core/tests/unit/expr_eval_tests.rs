use super::*;
use crate::run::RunState;

use std::collections::BTreeMap;
use std::path::PathBuf;

fn pipe_from(yaml: &str) -> Pipeline {
    serde_yaml::from_str(yaml).unwrap()
}

fn state_with(run_dir: PathBuf, inputs: serde_json::Value) -> RunState {
    let inputs_map: BTreeMap<String, serde_json::Value> = match inputs {
        serde_json::Value::Object(m) => m.into_iter().collect(),
        _ => BTreeMap::new(),
    };
    RunState {
        run_id: "test-run".into(),
        pipeline_path: run_dir.join("effective-pipeline.yaml"),
        pipeline_id: "t".into(),
        workspace_root: run_dir.clone(),
        run_dir,
        started_at: "now".into(),
        inputs: inputs_map,
        ..Default::default()
    }
}

fn tmp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("aidp-expr-{}-{}", tag, std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

const PIPE: &str = r#"
version: 4
id: t
inputs:
  track: { kind: enum, values: [quick, standard, deep], default: standard }
  publish: { kind: bool, default: false }
settings: { max_correction_iterations: 3 }
stages:
  - name: review
    outputs: [findings.json]
"#;

fn cond(expr: &str, state: &RunState, pipe: &Pipeline) -> Option<bool> {
    let ctx = EvalCtx::new(state, pipe);
    eval_condition(&serde_yaml::Value::String(expr.into()), &ctx)
}

#[test]
fn literal_true_false() {
    let pipe = pipe_from(PIPE);
    let st = state_with(tmp("lit"), serde_json::json!({}));
    assert_eq!(cond("true", &st, &pipe), Some(true));
    assert_eq!(cond("{{ false }}", &st, &pipe), Some(false));
}

#[test]
fn equality_against_input() {
    let pipe = pipe_from(PIPE);
    let st = state_with(tmp("eq"), serde_json::json!({"track": "deep"}));
    assert_eq!(cond("{{ inputs.track == 'deep' }}", &st, &pipe), Some(true));
    assert_eq!(
        cond("{{ inputs.track == 'quick' }}", &st, &pipe),
        Some(false)
    );
    assert_eq!(
        cond("{{ inputs.track != 'quick' }}", &st, &pipe),
        Some(true)
    );
}

#[test]
fn input_default_fallback() {
    // `track` omitted by the caller → resolves to the pipeline default.
    let pipe = pipe_from(PIPE);
    let st = state_with(tmp("def"), serde_json::json!({}));
    assert_eq!(
        cond("{{ inputs.track == 'standard' }}", &st, &pipe),
        Some(true)
    );
}

#[test]
fn membership() {
    let pipe = pipe_from(PIPE);
    let st = state_with(tmp("in"), serde_json::json!({"track": "standard"}));
    assert_eq!(
        cond("{{ inputs.track in ['standard','deep'] }}", &st, &pipe),
        Some(true)
    );
    let st2 = state_with(tmp("in2"), serde_json::json!({"track": "quick"}));
    assert_eq!(
        cond("{{ inputs.track in ['standard','deep'] }}", &st2, &pipe),
        Some(false)
    );
}

// The `{{ feature }}` token mirrors the run's grouping segment, so
// `feat/{{ feature }}` branch names and the `.aidp/pipelines/<feature>/`
// folder always agree — including for a multi-US run.
#[test]
fn feature_token_matches_run_grouping() {
    let pipe = pipe_from(PIPE);
    let render = |inputs: serde_json::Value| {
        let st = state_with(tmp("feat"), inputs);
        render_token("feature", &EvalCtx::new(&st, &pipe))
    };
    assert_eq!(
        render(serde_json::json!({"jira_keys": ["PIXEL-2", "PIXEL-1"]})),
        "PIXEL-1+PIXEL-2"
    );
    assert_eq!(
        render(serde_json::json!({"feature": "EPIC-9", "jira_keys": ["A-1"]})),
        "EPIC-9"
    );
    assert_eq!(
        render(serde_json::json!({"jira_key": "PIXEL-42"})),
        "PIXEL-42"
    );
    // Target-less → unknown ref stays visible.
    assert_eq!(render(serde_json::json!({})), "{{ feature }}");
}

#[test]
fn default_filter() {
    let pipe = pipe_from(PIPE);
    // publish absent and no value → default(false) → false.
    let st = state_with(tmp("dflt"), serde_json::json!({}));
    assert_eq!(
        cond("{{ inputs.publish | default(false) }}", &st, &pipe),
        Some(false)
    );
    let st2 = state_with(tmp("dflt2"), serde_json::json!({"publish": true}));
    assert_eq!(
        cond("{{ inputs.publish | default(false) }}", &st2, &pipe),
        Some(true)
    );
}

#[test]
fn reads_json_property_from_output_file() {
    let pipe = pipe_from(PIPE);
    let dir = tmp("json");
    // Write the producer's artifact with the property the condition reads.
    let review_dir = dir.join("review");
    std::fs::create_dir_all(&review_dir).unwrap();
    std::fs::write(
        review_dir.join("findings.json"),
        r#"{"has_blocking_findings": true}"#,
    )
    .unwrap();
    let st = state_with(dir, serde_json::json!({}));
    // This is the exact expression that previously resolved to a path string.
    assert_eq!(
        cond(
            "{{ stages.review.outputs.findings.has_blocking_findings }}",
            &st,
            &pipe
        ),
        Some(true),
    );
}

#[test]
fn output_ref_without_property_is_a_path() {
    let pipe = pipe_from(PIPE);
    let dir = tmp("path");
    let review_dir = dir.join("review");
    std::fs::create_dir_all(&review_dir).unwrap();
    std::fs::write(review_dir.join("findings.json"), "{}").unwrap();
    let st = state_with(dir, serde_json::json!({}));
    let ctx = EvalCtx::new(&st, &pipe);
    let rendered = render_token("stages.review.outputs.findings", &ctx);
    // Extension-correct path to the artifact that exists on disk.
    assert!(rendered.ends_with("findings.json"), "got: {rendered}");
}

#[test]
fn logical_and_or_not() {
    let pipe = pipe_from(PIPE);
    let st = state_with(tmp("logic"), serde_json::json!({"track": "deep"}));
    assert_eq!(
        cond(
            "{{ inputs.track == 'deep' and inputs.publish == false }}",
            &st,
            &pipe
        ),
        Some(true)
    );
    assert_eq!(
        cond(
            "{{ inputs.track == 'quick' or inputs.track == 'deep' }}",
            &st,
            &pipe
        ),
        Some(true)
    );
    assert_eq!(
        cond("{{ not inputs.track == 'quick' }}", &st, &pipe),
        Some(true)
    );
    assert_eq!(
        cond(
            "{{ inputs.track == 'quick' and inputs.track == 'deep' }}",
            &st,
            &pipe
        ),
        Some(false)
    );
}

#[test]
fn numeric_comparisons() {
    let pipe = pipe_from(PIPE);
    let st = state_with(tmp("cmp"), serde_json::json!({}));
    assert_eq!(
        cond("{{ settings.max_correction_iterations >= 3 }}", &st, &pipe),
        Some(true)
    );
    assert_eq!(
        cond("{{ settings.max_correction_iterations > 3 }}", &st, &pipe),
        Some(false)
    );
    assert_eq!(
        cond("{{ settings.max_correction_iterations < 5 }}", &st, &pipe),
        Some(true)
    );
    assert_eq!(
        cond("{{ settings.max_correction_iterations <= 3 }}", &st, &pipe),
        Some(true)
    );
}

// WI-P3-8: array indexing in JSON resolution + a length/count filter.
#[test]
fn array_index_and_length_filter() {
    // Pure index_json: a numeric segment indexes arrays; bad/oob → null.
    let doc = serde_json::json!({"items": [10, 20, 30]});
    assert_eq!(
        index_json(doc.clone(), &["items", "0"]),
        serde_json::json!(10)
    );
    assert_eq!(index_json(doc.clone(), &["items", "9"]), Json::Null);
    assert_eq!(index_json(doc, &["items", "nan"]), Json::Null);

    // length/count over an array read from an output artifact, and indexing.
    let pipe = pipe_from(PIPE);
    let dir = tmp("len");
    let review_dir = dir.join("review");
    std::fs::create_dir_all(&review_dir).unwrap();
    std::fs::write(
        review_dir.join("findings.json"),
        r#"{"items": ["a","b","c"]}"#,
    )
    .unwrap();
    let st = state_with(dir, serde_json::json!({}));
    assert_eq!(
        cond(
            "{{ stages.review.outputs.findings.items | length > 0 }}",
            &st,
            &pipe
        ),
        Some(true)
    );
    assert_eq!(
        cond(
            "{{ stages.review.outputs.findings.items | count == 3 }}",
            &st,
            &pipe
        ),
        Some(true)
    );
    assert_eq!(
        cond(
            "{{ stages.review.outputs.findings.items.0 == 'a' }}",
            &st,
            &pipe
        ),
        Some(true)
    );
}

#[test]
fn max_iterations_from_settings() {
    let pipe = pipe_from(PIPE);
    let st = state_with(tmp("maxiter"), serde_json::json!({}));
    let loopy: Stage = serde_yaml::from_str(
        "name: l\ntype: loop\nmax_iterations: \"{{ settings.max_correction_iterations }}\"\nbody: []",
    )
    .unwrap();
    assert_eq!(eval_max_iterations(&loopy, &st, &pipe), 3);
}

// A research (analyzer) stage's output resolves under the shared ticket root
// (run_dir's parent), not under this run's dir — so downstream refs point at
// the reused artifact.
#[test]
fn research_output_resolves_to_ticket_root() {
    let pipe = pipe_from(
        "version: 5\nid: r\nstages:\n  - name: explore\n    agent: analyzer\n    outputs: [exploration.md]\n",
    );
    // Lay out <base>/<ticket>/<run>; ticket root = <base>/<ticket>.
    let ticket_root = tmp("research").join("TICKET-1");
    let run_dir = ticket_root.join("run-x");
    std::fs::create_dir_all(&run_dir).unwrap();
    // The analyzer stage's artifact lives at the ticket root, under explore/.
    let research_dir = ticket_root.join("explore");
    std::fs::create_dir_all(&research_dir).unwrap();
    std::fs::write(research_dir.join("exploration.md"), "# notes").unwrap();

    let st = state_with(run_dir, serde_json::json!({}));
    let ctx = EvalCtx::new(&st, &pipe);
    let rendered = render_token("stages.explore.outputs.exploration", &ctx).replace('\\', "/");
    assert!(rendered.ends_with("exploration.md"), "got: {rendered}");
    assert!(
        rendered.contains("TICKET-1/explore"),
        "must resolve under the ticket root: {rendered}"
    );
    assert!(
        !rendered.contains("run-x"),
        "research artifact must not resolve under the run dir: {rendered}"
    );
}
