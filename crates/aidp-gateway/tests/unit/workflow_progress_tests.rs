use super::*;
use aidp_testkit::TempWorkspace;

const REAL_LAUNCH: &str = r#"Workflow launched in background. Task ID: wfd2utyf9
Summary: ai-dev-pipeline v7 - pr-comments (run 20260916-080908-cfd8)
Transcript dir: C:\Users\a.vuk-ext\.claude\projects\C--CODE--Net-cheat-aml-service\b9aa69bc-d05a-468e-87fc-893266eb2261\subagents\workflows\wf_6031c410-9e3
Script file: C:\CODE\AIDP\Pipelines\cheat-aml-service\488\20260916-080908-cfd8\workflow.js
(Edit this file with Write/Edit and re-invoke Workflow with {scriptPath: "..."} to iterate without resending the script.)
Run ID: wf_6031c410-9e3
To resume after ..."#;

#[test]
fn parses_real_workflow_launch() {
    let launch = parse_workflow_launch(REAL_LAUNCH).expect("should parse a real launch");
    assert_eq!(launch.run_id, "wf_6031c410-9e3");
    assert!(launch
        .transcript_dir
        .to_string_lossy()
        .ends_with("wf_6031c410-9e3"));
}

#[test]
fn rejects_plan_mode_refusal() {
    assert!(parse_workflow_launch("Review dynamic workflow before running").is_none());
}

#[test]
fn rejects_tool_use_error() {
    let content = "<tool_use_error>scriptPath must be a script path this tool returned, \
        not a hand-written path.</tool_use_error>";
    assert!(parse_workflow_launch(content).is_none());
}

#[test]
fn progress_from_line_sequence() {
    let mut cursor = JournalCursor::default();

    // `launched` names no agent and no phase.
    assert!(progress_from_line(&mut cursor, r#"{"type":"launched"}"#).is_none());

    // `started` with label/phase records both and reports them.
    let started = progress_from_line(
        &mut cursor,
        r#"{"type":"started","key":"v2:abc","agentId":"a1","label":"prepare_branch","phase":"prepare_branch"}"#,
    )
    .expect("started should produce a payload");
    assert_eq!(started["status"], "started");
    assert_eq!(started["agent"], "a1");
    assert_eq!(started["label"], "prepare_branch");
    assert_eq!(started["phase"], "prepare_branch");

    // A later `result` for the same agent inherits its phase/label from the cursor.
    let result = progress_from_line(
        &mut cursor,
        r#"{"type":"result","key":"v2:abc","agentId":"a1","result":{"status":"completed"}}"#,
    )
    .expect("result should produce a payload");
    assert_eq!(result["status"], "completed");
    assert_eq!(result["phase"], "prepare_branch");
    assert_eq!(result["label"], "prepare_branch");

    // `started` without label/phase (the 89% majority shape) yields null, not a panic.
    let bare_started = progress_from_line(
        &mut cursor,
        r#"{"type":"started","key":"v2:def","agentId":"a2"}"#,
    )
    .expect("bare started should still produce a payload");
    assert_eq!(bare_started["status"], "started");
    assert!(bare_started["phase"].is_null());
    assert!(bare_started["label"].is_null());

    // A `result` whose `result` value is a plain JSON string.
    let string_result = progress_from_line(
        &mut cursor,
        r#"{"type":"result","key":"v2:ghi","agentId":"a3","result":"ok"}"#,
    )
    .expect("string result should still produce a payload");
    assert_eq!(string_result["status"], "result");
    // a3 was never seen started.
    assert!(string_result["phase"].is_null());
    assert!(string_result["label"].is_null());

    // A `result` whose object carries no `status`.
    let statusless_result = progress_from_line(
        &mut cursor,
        r#"{"type":"result","key":"v2:jkl","agentId":"a2","result":{"summary":"done"}}"#,
    )
    .expect("statusless result should still produce a payload");
    assert_eq!(statusless_result["status"], "result");

    // `failed` carries no message/error field.
    let failed = progress_from_line(
        &mut cursor,
        r#"{"type":"failed","key":"v2:mno","agentId":"a2"}"#,
    )
    .expect("failed should produce a payload");
    assert_eq!(failed["status"], "failed");
    assert!(failed["phase"].is_null());

    // `failed` for an agent never seen started also yields null, not a panic.
    let unseen_failed = progress_from_line(&mut cursor, r#"{"type":"failed","agentId":"a4"}"#)
        .expect("failed for an unseen agent should still produce a payload");
    assert!(unseen_failed["phase"].is_null());
    assert!(unseen_failed["label"].is_null());

    // Blank lines and invalid JSON both yield `None`.
    assert!(progress_from_line(&mut cursor, "").is_none());
    assert!(progress_from_line(&mut cursor, "   ").is_none());
    assert!(progress_from_line(&mut cursor, "{not json").is_none());
}

fn unique_temp_dir(label: &str) -> TempWorkspace {
    TempWorkspace::new(&format!("workflow-progress-{label}"))
}

#[test]
fn drain_journal_holds_partial_line_across_polls() {
    use std::io::Write;

    let _ws = unique_temp_dir("drain");
    let dir = _ws.path();
    let journal = dir.join("journal.jsonl");

    // First append: one complete line, plus a partial one with no trailing newline.
    std::fs::write(
        &journal,
        b"{\"type\":\"launched\"}\n{\"type\":\"started\",\"agentId\":\"a1\"".as_slice(),
    )
    .expect("write first chunk");

    let mut cursor = JournalCursor::default();
    let first = drain_journal(&mut cursor, &journal);
    assert!(
        first.is_empty(),
        "the partial line must not be emitted on the first drain"
    );

    // Second append: finish the partial line.
    {
        let mut f = std::fs::OpenOptions::new()
            .append(true)
            .open(&journal)
            .expect("reopen journal for append");
        f.write_all(b",\"phase\":\"prepare_branch\"}\n")
            .expect("append rest of the line");
    }

    let second = drain_journal(&mut cursor, &journal);
    assert_eq!(second.len(), 1, "the completed line is emitted whole");
    assert_eq!(second[0]["status"], "started");
    assert_eq!(second[0]["agent"], "a1");
    assert_eq!(second[0]["phase"], "prepare_branch");

    let third = drain_journal(&mut cursor, &journal);
    assert!(third.is_empty(), "the line is emitted exactly once overall");
}

const RUN_ID: &str = "20261001-081734-9124";
const ENVELOPE: &str =
    r#"{"run_id":"20261001-081734-9124","run_dir":"C:/vault/run","script":"w.js"}"#;
const STATE_WITHOUT_RUN_DIR: &str = r#"{"run_id":"20261001-081734-9124","status":"running"}"#;
const ENVELOPE_WITHOUT_RUN_ID: &str = r#"{"run_dir":"C:/vault/run"}"#;
const MALFORMED_ID_ENVELOPE: &str = r#"{"run_id":"wf_6031c410-9e3","run_dir":"C:/vault/run"}"#;

/// The whole text being the envelope yields its run id.
#[test]
fn run_start_envelope_as_whole_text_is_parsed() {
    assert_eq!(parse_run_start_run_id(ENVELOPE), Some(RUN_ID.to_string()));
}

/// The envelope is found on its own line among other output.
#[test]
fn run_start_envelope_on_one_line_among_others_is_parsed() {
    let content = format!(
        "starting
{ENVELOPE}
done"
    );
    assert_eq!(parse_run_start_run_id(&content), Some(RUN_ID.to_string()));
}

/// Each text block of an array result is searched.
#[test]
fn run_start_envelope_in_text_block_of_array_result_is_parsed() {
    let blocks = serde_json::json!([
        { "type": "text", "text": "no envelope here" },
        { "type": "text", "text": ENVELOPE }
    ]);
    assert_eq!(
        parse_run_start_run_id(&blocks.to_string()),
        Some(RUN_ID.to_string())
    );
}

/// A run id that does not have the aidp shape is rejected.
#[test]
fn run_start_envelope_with_malformed_run_id_is_rejected() {
    assert_eq!(parse_run_start_run_id(MALFORMED_ID_ENVELOPE), None);
}

/// An object without `run_id` yields nothing.
#[test]
fn run_start_envelope_without_run_id_is_rejected() {
    assert_eq!(parse_run_start_run_id(ENVELOPE_WITHOUT_RUN_ID), None);
}

/// A state-like object carrying `run_id` but no `run_dir` is not a start envelope.
#[test]
fn run_state_like_json_without_run_dir_is_rejected() {
    assert_eq!(parse_run_start_run_id(STATE_WITHOUT_RUN_DIR), None);
}

/// Well-formed and malformed run ids.
#[test]
fn is_aidp_run_id_accepts_only_the_aidp_shape() {
    assert!(is_aidp_run_id(RUN_ID));
    assert!(is_aidp_run_id("20261001-081734-abcd"));
    assert!(!is_aidp_run_id("2026100-081734-9124"));
    assert!(!is_aidp_run_id("20261001-08173-9124"));
    assert!(!is_aidp_run_id("20261001-081734-"));
    assert!(!is_aidp_run_id("20261001-081734-ABCD"));
    assert!(!is_aidp_run_id("20261001-081734"));
    assert!(!is_aidp_run_id("wf_6031c410-9e3"));
}
