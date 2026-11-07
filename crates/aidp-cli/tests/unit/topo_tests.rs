use super::*;
use crate::commands::Payload;
use std::path::PathBuf;

const MINIMAL_FIXTURE: &str = "tests/fixtures/t05-minimal.yaml";

fn test_ctx() -> Ctx {
    Ctx::for_test(PathBuf::from("."), None)
}

#[test]
fn topo_returns_the_layers_instead_of_printing_them() {
    let output = run(
        &test_ctx(),
        TopoArgs {
            pipeline: MINIMAL_FIXTURE.to_string(),
        },
    )
    .expect("a minimal pipeline sorts");

    let Payload::Text(text) = output.payload else {
        panic!("topo must hand its layers back as text");
    };
    let layers: Vec<Vec<String>> = serde_json::from_str(&text).unwrap();
    assert_eq!(layers, vec![vec!["only".to_string()]]);
    assert!(output.diagnostics.is_empty());
    assert!(output.failure.is_none());
    assert!(output.exit.is_none());
}

#[test]
fn topo_fails_for_an_unknown_pipeline() {
    let result = run(
        &test_ctx(),
        TopoArgs {
            pipeline: "no-such-pipeline-id".to_string(),
        },
    );
    assert!(result.is_err());
}
