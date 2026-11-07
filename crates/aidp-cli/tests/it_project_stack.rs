//! `aidp project stack` on real directories.

use crate::common;
use common::{aidp, workspace};
use std::fs;

const PROJECT_ARG: &str = "project";
const PROJECT_DIR: &str = "project";
const STACK_ARG: &str = "stack";
const STACK_KEY: &str = "stack";
const SOLUTION_FILE: &str = "app.sln";
const PACKAGE_FILE: &str = "package.json";
const SERVERLESS_FILE: &str = "serverless.yml";
const ANGULAR_FILE: &str = "angular.json";
const CARGO_FILE: &str = "Cargo.toml";
const FILE_BODY: &str = "{}";
const CSHARP_STACK: &str = "csharp";
const LAMBDA_TS_STACK: &str = "lambda-ts";
const ANGULAR_STACK: &str = "angular";
const RUST_STACK: &str = "rust";
const TS_NODE_STACK: &str = "ts-node";
const UNKNOWN_STACK: &str = "unknown";
const MISSING_DIR: &str = "absent";
const NOT_FOUND_EXIT: i32 = 3;
const SUCCESS_EXIT: i32 = 0;

fn detect(files: &[&str]) -> (Option<i32>, serde_json::Value) {
    let ws = workspace();
    let project = ws.child(PROJECT_DIR);
    fs::create_dir_all(&project).unwrap();
    for file in files {
        fs::write(project.join(file), FILE_BODY).unwrap();
    }
    let output = aidp(&ws)
        .args([PROJECT_ARG, STACK_ARG])
        .arg(&project)
        .output()
        .unwrap();
    let report = serde_json::from_slice(&output.stdout).unwrap_or_default();
    (output.status.code(), report)
}

#[test]
fn it_project_stack_reports_csharp_for_a_solution() {
    let (code, report) = detect(&[SOLUTION_FILE]);

    assert_eq!(code, Some(SUCCESS_EXIT));
    assert_eq!(report[STACK_KEY], CSHARP_STACK);
}

#[test]
fn it_project_stack_reports_lambda_ts_for_a_serverless_package() {
    let (code, report) = detect(&[PACKAGE_FILE, SERVERLESS_FILE]);

    assert_eq!(code, Some(SUCCESS_EXIT));
    assert_eq!(report[STACK_KEY], LAMBDA_TS_STACK);
}

#[test]
fn it_project_stack_reports_angular_for_an_angular_manifest() {
    let (code, report) = detect(&[ANGULAR_FILE]);

    assert_eq!(code, Some(SUCCESS_EXIT));
    assert_eq!(report[STACK_KEY], ANGULAR_STACK);
}

#[test]
fn it_project_stack_reports_rust_for_a_cargo_manifest() {
    let (code, report) = detect(&[CARGO_FILE]);

    assert_eq!(code, Some(SUCCESS_EXIT));
    assert_eq!(report[STACK_KEY], RUST_STACK);
}

#[test]
fn it_project_stack_reports_ts_node_for_a_bare_package() {
    let (code, report) = detect(&[PACKAGE_FILE]);

    assert_eq!(code, Some(SUCCESS_EXIT));
    assert_eq!(report[STACK_KEY], TS_NODE_STACK);
}

#[test]
fn it_project_stack_reports_unknown_without_any_marker() {
    let (code, report) = detect(&[]);

    assert_eq!(code, Some(SUCCESS_EXIT));
    assert_eq!(report[STACK_KEY], UNKNOWN_STACK);
}

#[test]
fn it_project_stack_exits_not_found_for_a_missing_directory() {
    let ws = workspace();
    let output = aidp(&ws)
        .args([PROJECT_ARG, STACK_ARG])
        .arg(ws.child(MISSING_DIR))
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(NOT_FOUND_EXIT));
}
