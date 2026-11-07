use super::*;

const LOOP_BODY_CYCLE: &str = include_str!("../fixtures/loop/loop-body-cycle.yaml");
const LOOP_BODY_UNKNOWN_DEP: &str = include_str!("../fixtures/loop/loop-body-unknown-dep.yaml");
const LOOP_BODY_VALID_DEPS: &str = include_str!("../fixtures/loop/loop-body-valid-deps.yaml");

const LOOP_STAGE_NAME: &str = "correction";
const CYCLE_ERROR_FRAGMENT: &str = "cycle or unresolved dep";
const BODY_STAGE_REFIX: &str = "refix";
const BODY_STAGE_REPLAN: &str = "replan";

/// Writes `yaml` to a fresh temp file and loads it as a [`Pipeline`].
fn load_fixture(tag: &str, yaml: &str) -> Pipeline {
    let dir = std::env::temp_dir().join(format!("aidp-pipeline-loop-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("pipe.yaml");
    std::fs::write(&path, yaml).unwrap();
    let pipe = load(&path).expect("the fixture must pass schema validation");
    let _ = std::fs::remove_dir_all(&dir);
    pipe
}

/// A cycle confined to a loop's `body` must be refused by `topo_sort` the same
/// way a top-level cycle is, naming both stages and the loop.
#[test]
fn topo_sort_rejects_a_cyclic_loop_body_naming_both_stages() {
    let pipe = load_fixture("cycle", LOOP_BODY_CYCLE);
    let error = topo_sort(&pipe)
        .expect_err("a cyclic loop body must be refused")
        .to_string();
    assert!(error.contains(LOOP_STAGE_NAME), "{error}");
    assert!(error.contains(CYCLE_ERROR_FRAGMENT), "{error}");
    assert!(error.contains(BODY_STAGE_REFIX), "{error}");
    assert!(error.contains(BODY_STAGE_REPLAN), "{error}");
}

/// A body stage's `needs` naming neither a sibling body stage nor a top-level
/// stage is an unresolved dep, refused the same way a cycle is.
#[test]
fn topo_sort_rejects_a_loop_body_needs_on_an_unknown_stage() {
    let pipe = load_fixture("unknown-dep", LOOP_BODY_UNKNOWN_DEP);
    let error = topo_sort(&pipe)
        .expect_err("a body `needs` on an unknown stage must be refused")
        .to_string();
    assert!(error.contains(LOOP_STAGE_NAME), "{error}");
    assert!(error.contains(CYCLE_ERROR_FRAGMENT), "{error}");
    assert!(error.contains(BODY_STAGE_REFIX), "{error}");
}

/// A body stage's `needs` may name a top-level stage or an earlier sibling —
/// both are accepted, and the top-level layers are unaffected by the body
/// check.
#[test]
fn topo_sort_accepts_a_loop_body_needs_on_a_top_level_stage_and_an_earlier_sibling() {
    let pipe = load_fixture("valid-deps", LOOP_BODY_VALID_DEPS);
    let layers = topo_sort(&pipe).expect("a valid loop body graph must pass");
    let top_level_names: Vec<&str> = layers.iter().flatten().map(String::as_str).collect();
    assert_eq!(
        top_level_names,
        vec!["gate", LOOP_STAGE_NAME],
        "the loop body check must not drop or reorder the top-level stages"
    );
}

/// A malformed pipeline YAML must fail with a message naming the file, and
/// its cause chain must still carry the underlying `serde_yaml::Error` — a
/// planner reading only the top line still needs the file identified, and a
/// human debugging the cause needs the original parser error preserved.
#[test]
fn load_on_malformed_yaml_names_the_file_and_keeps_the_yaml_error_in_the_chain() {
    let dir = std::env::temp_dir().join(format!("aidp-pipeline-badyaml-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("broken-pipeline.yaml");
    std::fs::write(&path, "version: 5\nid: [unterminated\n").unwrap();

    let error = load(&path).expect_err("malformed yaml must not load");
    let rendered = format!("{error:#}");
    assert!(rendered.contains("broken-pipeline.yaml"), "{rendered}");
    assert!(
        error
            .chain()
            .any(|c| c.downcast_ref::<serde_yaml::Error>().is_some()),
        "{rendered}"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// An undeclared input must produce a warning naming that input's key,
/// without failing validation.
#[test]
fn validate_inputs_collect_warns_on_an_undeclared_input_key() {
    let dir = std::env::temp_dir().join(format!(
        "aidp-pipeline-undeclared-input-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("pipe.yaml");
    std::fs::write(
        &path,
        "version: 5
id: undeclared-input
stages:
  - name: a
    agent: analyzer
",
    )
    .unwrap();
    let pipe = load(&path).expect("the fixture must pass schema validation");
    let _ = std::fs::remove_dir_all(&dir);

    let mut provided = std::collections::BTreeMap::new();
    provided.insert("mystery_key".to_string(), serde_json::json!("x"));
    let mut warnings = Vec::new();
    let out = validate_inputs_collect(&pipe, &provided, &mut warnings)
        .expect("an undeclared input must not fail validation");
    assert_eq!(out.get("mystery_key"), Some(&serde_json::json!("x")));
    assert!(
        warnings.iter().any(|w| w.contains("mystery_key")),
        "{warnings:?}"
    );
}

/// The compiled JSON-Schema is memoized: every `load` call reuses the same
/// compiled instance instead of building a new one.
#[test]
fn schema_compiles_at_most_once_across_several_load_calls() {
    let mut compiled_schema_addresses = Vec::new();
    for i in 0..3 {
        let dir = std::env::temp_dir().join(format!(
            "aidp-pipeline-schema-memo-{i}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("pipe.yaml");
        std::fs::write(
            &path,
            "version: 5
id: schema-memo
stages:
  - name: a
    agent: analyzer
",
        )
        .unwrap();
        let _ = load(&path).expect("the fixture must pass schema validation");
        let compiled_schema = Lazy::get(&COMPILED_SCHEMA).expect("load must compile the schema");
        compiled_schema_addresses.push(std::ptr::from_ref(compiled_schema));
        let _ = std::fs::remove_dir_all(&dir);
    }
    assert!(
        compiled_schema_addresses
            .windows(2)
            .all(|pair| pair[0] == pair[1]),
        "the schema must be compiled at most once per process"
    );
}
