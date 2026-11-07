use crate::common;
use common::*;
use tempfile::TempDir;

const UNKNOWN_RUN_EXIT: i32 = 3;

#[test]
fn plan_exec_writes_execution_plan() {
    let ws = workspace();
    let _tmp = TempDir::new().expect("tempdir");
    let init = aidp_with_fixture_skill(&ws)
        .args(["run", "init", "--pipeline"])
        .arg(skill_root().join("templates/dev-aidp.yaml"))
        .args(["--no-vault", "--workspace-root", ".", "--inputs"])
        .arg(r#"{"prompt":"p","feature":"filet","track":"quick"}"#)
        .output()
        .expect("run init");
    assert!(init.status.success());
    let run_id = String::from_utf8_lossy(&init.stdout).trim().to_string();

    let out = aidp_with_fixture_skill(&ws)
        .args([
            "run",
            "plan-exec",
            "--run",
            &run_id,
            "--no-vault",
            "--workspace-root",
            ".",
        ])
        .output()
        .expect("plan-exec");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let parsed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("plan file")).expect("json");
    assert!(parsed["steps"].as_array().is_some_and(|s| !s.is_empty()));
}

#[test]
fn plan_exec_unknown_run_exits_not_found() {
    let ws = workspace();
    let out = aidp_with_fixture_skill(&ws)
        .args([
            "run",
            "plan-exec",
            "--run",
            "no-such-run",
            "--no-vault",
            "--workspace-root",
            ".",
        ])
        .output()
        .expect("plan-exec");
    assert_eq!(out.status.code(), Some(UNKNOWN_RUN_EXIT));
}
