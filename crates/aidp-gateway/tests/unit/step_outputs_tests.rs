use super::*;
use crate::models::native_run_view::NativeRunView;
use serde_json::json;

const VIEW_FIXTURE: &str = include_str!("../fixtures/flows/native_run_view.json");

fn view(dir: &std::path::Path, stages: Value) -> NativeRunView {
    let mut value: Value = serde_json::from_str(VIEW_FIXTURE).unwrap();
    value["runDir"] = json!(dir.to_string_lossy());
    value["stages"] = stages;
    serde_json::from_value(value).unwrap()
}

fn row(name: &str, status: &str, finished: &str, signals: Option<Value>) -> Value {
    let mut row = json!({"name": name, "status": status, "finishedAt": finished});
    if let Some(signals) = signals {
        row["signals"] = signals;
    }
    row
}

fn write_result(dir: &std::path::Path, stage: &str, text: &str) {
    std::fs::create_dir_all(dir.join(stage)).unwrap();
    std::fs::write(dir.join(stage).join(RESULT_FILE), text).unwrap();
}

#[test]
fn result_json_of_the_last_stage_gives_typed_outputs() {
    let dir = tempfile::tempdir().unwrap();
    write_result(dir.path(), "first", r#"{"verdict":"old"}"#);
    write_result(
        dir.path(),
        "last",
        r#"{"verdict":"done","passed":true,"n":3,"list":[1],"obj":{"a":1}}"#,
    );
    let stages = json!([
        row("last", "done", "2026-01-01T00:00:02Z", None),
        row("first", "done", "2026-01-01T00:00:01Z", None),
    ]);
    let outputs = last_stage_outputs(&view(dir.path(), stages));
    assert_eq!(outputs["verdict"], json!("done"));
    assert_eq!(outputs["passed"], json!(true));
    assert_eq!(outputs["n"], json!(3));
    assert_eq!(outputs["list"], json!([1]));
    assert_eq!(outputs["obj"], json!({"a": 1}));
}

#[test]
fn signals_are_the_fallback_when_result_json_is_missing() {
    let dir = tempfile::tempdir().unwrap();
    let stages = json!([row("only", "done", "t", Some(json!({"verdict": "done"})))]);
    let outputs = last_stage_outputs(&view(dir.path(), stages));
    assert_eq!(outputs["verdict"], json!("done"));
}

#[test]
fn blocked_reason_comes_from_signals_when_result_json_lacks_it() {
    let dir = tempfile::tempdir().unwrap();
    write_result(dir.path(), "s", r#"{"passed":false}"#);
    let stages = json!([row(
        "s",
        "failed",
        "t",
        Some(json!({"blocked_reason": "no license"}))
    )]);
    let outputs = last_stage_outputs(&view(dir.path(), stages));
    assert_eq!(outputs[OUTPUT_BLOCKED_REASON], json!("no license"));
    assert_eq!(outputs["passed"], json!(false));
}

#[test]
fn shards_are_not_the_last_stage_and_no_stage_gives_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let stages = json!([
        row(
            "real",
            "done",
            "2026-01-01T00:00:01Z",
            Some(json!({"k": 1}))
        ),
        row(
            "real#T1",
            "done",
            "2026-01-01T00:00:09Z",
            Some(json!({"k": 2}))
        ),
    ]);
    assert_eq!(last_stage_outputs(&view(dir.path(), stages))["k"], json!(1));
    assert!(last_stage_outputs(&view(dir.path(), json!([]))).is_empty());
}

#[test]
fn pr_number_wins_and_file_paths_move_to_a_file_key() {
    let dir = tempfile::tempdir().unwrap();
    write_result(
        dir.path(),
        "s",
        r#"{"pr":7,"findings":[],"runId":"x","pr_body":"text"}"#,
    );
    let stages = json!([row("s", "done", "t", None)]);
    let mut outputs = BTreeMap::from([
        ("pr_file".to_string(), json!("/p/pr.json")),
        ("pr_body".to_string(), json!("/p/pr-body.md")),
        ("findings".to_string(), json!("/p/findings.json")),
        ("runId".to_string(), json!("run-1")),
    ]);
    merge_stage_outputs(&mut outputs, &view(dir.path(), stages));
    assert_eq!(outputs["pr"], json!(7));
    assert_eq!(outputs["pr_file"], json!("/p/pr.json"));
    assert_eq!(outputs["pr_body"], json!("/p/pr-body.md"));
    assert_eq!(outputs["findings"], json!([]));
    assert_eq!(outputs["findings_file"], json!("/p/findings.json"));
    assert_eq!(outputs["runId"], json!("run-1"));
}
