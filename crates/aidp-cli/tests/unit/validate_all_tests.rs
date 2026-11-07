use super::*;
use aidp_testkit::TempWorkspace;

#[test]
fn validate_pipeline_file_ok_for_minimal_valid_pipeline() {
    let workspace = TempWorkspace::new("validate-ok");
    let dir = workspace.path();
    let path = dir.join("min.yaml");
    std::fs::write(
        &path,
        "version: 5\nid: min\nstages:\n  - name: only\n    agent: executor\n",
    )
    .unwrap();

    let out = validate_pipeline_file(&path, dir, None)
        .expect("a minimal well-formed pipeline must validate");
    assert_eq!(out["ok"], serde_json::json!(true));
}

#[test]
fn validate_pipeline_file_errs_for_missing_stages() {
    let workspace = TempWorkspace::new("validate-bad");
    let dir = workspace.path();
    let path = dir.join("bad.yaml");
    std::fs::write(&path, "version: 5\nid: bad\n").unwrap();

    assert!(
        validate_pipeline_file(&path, dir, None).is_err(),
        "a pipeline missing the required `stages:` field must fail validation, not silently pass"
    );
}

/// Regression test for the `--all` bug: the `Validate` match arm used to
/// bind `all: _` (discarding the flag) and unconditionally require
/// `pipeline`, so `aidp validate --all` always failed with "pipeline id
/// or path required" instead of validating every discovered template.
/// This exercises the same discovery-then-validate loop the `all` branch
/// now runs — `aidp_core::pipeline::discover` (the same call `aidp
/// pipeline list` makes) followed by `validate_pipeline_file` per
/// file — across a mix of one valid and one invalid template.
#[test]
fn it_all_discovers_and_reports_every_template_without_requiring_pipeline_arg() {
    let workspace = TempWorkspace::new("validate-discover");
    let dir = workspace.path();
    std::fs::write(
        dir.join("good.yaml"),
        "version: 5\nid: discover-good\nstages:\n  - name: only\n    agent: executor\n",
    )
    .unwrap();
    std::fs::write(dir.join("bad.yaml"), "version: 5\nid: discover-bad\n").unwrap();

    let refs: Vec<&Path> = vec![dir];
    let found = aidp_core::pipeline::discover(&refs).unwrap();
    assert_eq!(
        found.len(),
        2,
        "both templates must be discovered by id, mirroring `aidp pipeline list`"
    );

    let failed = found
        .iter()
        .filter(|(_, path)| validate_pipeline_file(path, dir, None).is_err())
        .count();
    assert_eq!(
        failed, 1,
        "exactly the schema-invalid template must fail, the valid one must not"
    );
}
