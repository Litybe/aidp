use super::*;
use serde_json::json;

fn context() -> Context {
    Context::new(json!({
        "steps": {
            "build": {"status": "done", "outputs": {"green": true, "count": 3}},
            "review": {"status": "waiting_human", "outputs": {"decision": "approve"}}
        },
        "settings": {"limit": 2, "mode": "strict"},
        "inputs": {"flag": false},
        "ticket": {"source": "local", "key": "LOC-1"}
    }))
}

#[test]
fn evaluates_table_of_expressions() {
    let cases = [
        ("steps.build.outputs.green == true", true),
        ("steps.build.outputs.green != true", false),
        ("steps.build.status == 'done'", true),
        ("steps.review.outputs.decision == \"approve\"", true),
        ("steps.build.outputs.count > 2", true),
        ("steps.build.outputs.count < 3", false),
        ("steps.build.outputs.count >= 3", true),
        ("steps.build.outputs.count <= 2", false),
        ("settings.limit == 2", true),
        ("settings.limit == 2.0", true),
        ("settings.mode == 'strict' && inputs.flag == false", true),
        ("inputs.flag || ticket.source == 'local'", true),
        ("!inputs.flag", true),
        ("!(settings.limit == 2)", false),
        (
            "(inputs.flag || settings.limit > 1) && ticket.key == 'LOC-1'",
            true,
        ),
        ("steps.missing.outputs.x == null", true),
        ("steps.missing.status == 'done'", false),
        ("steps.missing.outputs.n > 1", false),
        ("true && !false", true),
        ("-1 < 0", true),
    ];

    for (source, expected) in cases {
        let parsed = parse(source).unwrap_or_else(|error| panic!("{source}: {error}"));
        assert_eq!(parsed.eval_bool(&context()).unwrap(), expected, "{source}");
    }
}

#[test]
fn or_binds_looser_than_and() {
    let parsed = parse("true || false && false").unwrap();

    assert!(parsed.eval_bool(&context()).unwrap());
}

#[test]
fn parse_errors_are_reported() {
    let cases = [
        "",
        "steps.a ==",
        "(steps.a == 1",
        "steps.a == 'open",
        "steps..a == 1",
        "a = b",
        "1 2",
        "== 1",
    ];

    for source in cases {
        assert!(
            matches!(parse(source), Err(FlowError::Parse(_))),
            "`{source}` should not parse"
        );
    }
}

#[test]
fn ordering_mixed_types_is_an_evaluation_error() {
    let parsed = parse("settings.mode > 1").unwrap();

    assert!(matches!(parsed.eval(&context()), Err(FlowError::Eval(_))));
}

#[test]
fn step_refs_lists_each_referenced_step_once() {
    let parsed = parse(
        "steps.build.outputs.green == true && steps.build.status == 'done' || steps.review.status == 'x' || settings.limit > 1",
    )
    .unwrap();

    assert_eq!(parsed.step_refs(), vec!["build", "review"]);
}

#[test]
fn truthiness_follows_json_shape() {
    assert!(!is_truthy(&json!(null)));
    assert!(!is_truthy(&json!(0)));
    assert!(!is_truthy(&json!("")));
    assert!(!is_truthy(&json!([])));
    assert!(is_truthy(&json!("x")));
    assert!(is_truthy(&json!({"a": 1})));
}

#[test]
fn lookup_walks_arrays_by_index() {
    let context = Context::new(json!({"ticket": {"labels": ["a", "b"]}}));
    let parsed = parse("ticket.labels.1 == 'b'").unwrap();

    assert!(parsed.eval_bool(&context).unwrap());
}
