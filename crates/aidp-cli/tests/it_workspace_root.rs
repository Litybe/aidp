//! `--workspace-root` decides where project-local pipelines are discovered,
//! whatever the current directory is.

use crate::common;
use common::*;
use std::fs;

/// Creates a workspace-root directory holding `demo` under `.aidp/pipeline/flow/`.
fn root_with_demo_pipeline() -> aidp_testkit::TempWorkspace {
    let root = aidp_testkit::TempWorkspace::new("workspace-root-target");
    let flow_dir = root.child(".aidp/pipeline/flow");
    fs::create_dir_all(&flow_dir).expect("create flow dir");
    let fixture = crate_dir().join("tests/fixtures/workspace_root/demo.yaml");
    fs::copy(fixture, flow_dir.join("demo.yaml")).expect("copy demo pipeline");
    root
}

/// Validate resolves `demo` from the workspace root, not from the empty cwd.
#[test]
fn validate_uses_workspace_root_over_cwd() {
    let root = root_with_demo_pipeline();
    let cwd = workspace();
    let output = aidp(&cwd)
        .args(["validate", "demo", "--workspace-root"])
        .arg(root.path())
        .output()
        .expect("run aidp");
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(output.status.code(), Some(0), "stdout: {stdout}");
    assert!(stdout.contains("demo.yaml"), "stdout: {stdout}");
    assert!(stdout.contains("flow"), "stdout: {stdout}");
}

/// Run init resolves `demo` through the workspace root as well.
#[test]
fn run_init_resolves_pipeline_from_workspace_root() {
    let root = root_with_demo_pipeline();
    let cwd = workspace();
    let output = aidp(&cwd)
        .args([
            "run",
            "init",
            "--pipeline",
            "demo",
            "--target",
            "t",
            "--no-vault",
            "--workspace-root",
        ])
        .arg(root.path())
        .output()
        .expect("run aidp");
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
