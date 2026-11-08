use super::*;
use aidp_core::models::enums::agent_role::AgentRole;
use std::path::PathBuf;

const OUTPUT_REPORT: &str = "report";

fn stage_dir_with(fixture: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src/tests/fixtures/native_reply")
        .join(fixture);
    std::fs::copy(source, dir.path().join(REPLY_FILE)).unwrap();
    dir
}

fn declared() -> Vec<String> {
    vec![OUTPUT_REPORT.to_string()]
}

#[test]
fn it_valid_envelope_is_accepted() {
    let dir = stage_dir_with("valid.md");
    let envelope = read_reply(dir.path(), AgentRole::Reviewer, &declared()).unwrap();
    assert_eq!(envelope.status, "completed");
}

#[test]
fn it_fenced_envelope_is_accepted() {
    let dir = stage_dir_with("fenced.md");
    assert!(read_reply(dir.path(), AgentRole::Analyzer, &declared()).is_ok());
}

#[test]
fn missing_file_is_reported() {
    let dir = tempfile::tempdir().unwrap();
    let err = read_reply(dir.path(), AgentRole::Analyzer, &[]).unwrap_err();
    assert!(matches!(err, NativeReplyError::Missing(_)));
}

#[test]
fn it_invalid_json_is_reported() {
    let dir = stage_dir_with("invalid_json.md");
    let err = read_reply(dir.path(), AgentRole::Analyzer, &[]).unwrap_err();
    assert!(matches!(err, NativeReplyError::InvalidJson(_)));
}

#[test]
fn it_missing_declared_output_is_reported() {
    let dir = stage_dir_with("missing_output.md");
    let err = read_reply(dir.path(), AgentRole::Analyzer, &declared()).unwrap_err();
    assert!(matches!(err, NativeReplyError::MissingOutput(name) if name == OUTPUT_REPORT));
}

#[test]
fn it_reviewer_without_blocking_signal_is_rejected() {
    let dir = stage_dir_with("reviewer_missing_signal.md");
    let err = read_reply(dir.path(), AgentRole::Reviewer, &[]).unwrap_err();
    assert!(matches!(err, NativeReplyError::ReviewerSignals));
}

fn stage_dir_with_reply(reply: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(REPLY_FILE), reply).unwrap();
    dir
}

/// Agents key an output by the file name the template lists (`report.md`); it answers for `report`.
#[test]
fn an_output_keyed_by_its_file_name_answers_for_the_declared_output() {
    let dir = stage_dir_with_reply(
        r#"{"status":"completed","summary":"s","outputs":{"report.md":"pr-42"},"signals":{}}"#,
    );
    let envelope = read_reply(dir.path(), AgentRole::Analyzer, &declared()).unwrap();
    assert_eq!(
        envelope.outputs.get(OUTPUT_REPORT).map(String::as_str),
        Some("pr-42")
    );
    assert!(!envelope.outputs.contains_key("report.md"));
}

/// A key naming another file is still no answer for the declared output.
#[test]
fn an_output_for_another_file_still_misses_the_declared_output() {
    let dir = stage_dir_with_reply(
        r#"{"status":"completed","summary":"s","outputs":{"reports.md":"pr-42"},"signals":{}}"#,
    );
    let err = read_reply(dir.path(), AgentRole::Analyzer, &declared()).unwrap_err();
    assert!(matches!(err, NativeReplyError::MissingOutput(name) if name == OUTPUT_REPORT));
}

fn reply_with_keys(dir: &tempfile::TempDir, keys: &[&str]) {
    let outputs: Vec<String> = keys.iter().map(|k| format!(r#""{k}":"x""#)).collect();
    let reply = format!(
        r#"{{"status":"completed","summary":"s","outputs":{{{}}},"signals":{{}}}}"#,
        outputs.join(",")
    );
    std::fs::write(dir.path().join(REPLY_FILE), reply).unwrap();
}

#[test]
fn underscore_key_answers_for_dashed_declared_output() {
    let dir = tempfile::tempdir().unwrap();
    reply_with_keys(&dir, &["context", "memory_context"]);
    let declared = vec!["context".to_string(), "memory-context".to_string()];
    let envelope = read_reply(dir.path(), AgentRole::Analyzer, &declared).unwrap();
    assert!(envelope.outputs.contains_key("memory-context"));
    assert!(!envelope.outputs.contains_key("memory_context"));
}

#[test]
fn underscore_key_with_other_case_and_extension_answers_for_dashed_output() {
    let dir = tempfile::tempdir().unwrap();
    reply_with_keys(&dir, &["Pixellab_Usage.json"]);
    let declared = vec!["pixellab-usage".to_string()];
    let envelope = read_reply(dir.path(), AgentRole::Analyzer, &declared).unwrap();
    assert_eq!(
        envelope.outputs.get("pixellab-usage").map(String::as_str),
        Some("x")
    );
}

#[test]
fn declared_file_name_with_extension_matches_underscore_key() {
    let dir = tempfile::tempdir().unwrap();
    reply_with_keys(&dir, &["pixellab_usage"]);
    let declared = vec!["pixellab-usage.json".to_string()];
    let envelope = read_reply(dir.path(), AgentRole::Analyzer, &declared).unwrap();
    assert!(envelope.outputs.contains_key("pixellab-usage.json"));
}

#[test]
fn non_empty_file_without_key_is_accepted() {
    let dir = tempfile::tempdir().unwrap();
    reply_with_keys(&dir, &["other"]);
    std::fs::write(dir.path().join("memory-context.md"), "notes").unwrap();
    let declared = vec!["memory-context".to_string()];
    let envelope = read_reply(dir.path(), AgentRole::Analyzer, &declared).unwrap();
    let path = envelope.outputs.get("memory-context").unwrap();
    assert!(path.ends_with("memory-context.md"));
}

#[test]
fn empty_file_without_key_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    reply_with_keys(&dir, &["other"]);
    std::fs::write(dir.path().join("memory-context.md"), "").unwrap();
    let declared = vec!["memory-context".to_string()];
    let err = read_reply(dir.path(), AgentRole::Analyzer, &declared).unwrap_err();
    assert!(matches!(err, NativeReplyError::MissingOutput(n) if n == "memory-context"));
}

#[test]
fn absent_file_without_key_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    reply_with_keys(&dir, &["other"]);
    let declared = vec!["memory-context".to_string()];
    let err = read_reply(dir.path(), AgentRole::Analyzer, &declared).unwrap_err();
    assert!(matches!(err, NativeReplyError::MissingOutput(n) if n == "memory-context"));
}

#[test]
fn repair_instruction_names_the_error() {
    let text = repair_instruction(&NativeReplyError::MissingOutput("plan".into()));
    assert!(text.contains("plan") && text.contains(REPLY_FILE));
}

const WINDOWS_PATH_REPLY: &str =
    r#"{"status":"completed","summary":"s","outputs":{"a":"C:\Users\x\a.md"},"signals":{}}"#;
const WINDOWS_PATH: &str = r"C:\Users\x\a.md";

/// A reply holding unescaped Windows paths is still read.
#[test]
fn unescaped_windows_paths_are_escaped_and_read() {
    let value = extract_json(WINDOWS_PATH_REPLY).unwrap();
    assert_eq!(value["outputs"]["a"], WINDOWS_PATH);
}

/// A valid escape is left alone.
#[test]
fn valid_escapes_are_kept() {
    let text = r#"{"a":"x\ny\\z"}"#;
    assert_eq!(escape_stray_backslashes(text), text);
}

/// `\f` and `\t` in a Windows output path decode to control characters; the file on disk is still found.
#[cfg(windows)]
#[test]
fn an_output_path_with_decoded_backslash_escapes_is_restored() {
    let dir = tempfile::tempdir().unwrap();
    let stage = dir.path().join("forge").join("tests");
    std::fs::create_dir_all(&stage).unwrap();
    let file = stage.join("report.md");
    std::fs::write(&file, "r").unwrap();
    let expected = file.display().to_string();
    let written = expected
        .replace(r"\forge", "\u{c}orge")
        .replace(r"\tests", "\tests");
    let reply = serde_json::json!({
        "status": "completed", "summary": "s", "outputs": { OUTPUT_REPORT: written }, "signals": {}
    });
    std::fs::write(dir.path().join(REPLY_FILE), reply.to_string()).unwrap();
    let envelope = read_reply(dir.path(), AgentRole::Analyzer, &declared()).unwrap();
    assert_eq!(
        envelope.outputs.get(OUTPUT_REPORT).map(String::as_str),
        Some(expected.as_str())
    );
}

#[test]
fn reply_with_a_utf8_bom_is_accepted() {
    let dir = tempfile::tempdir().unwrap();
    let source =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/tests/fixtures/native_reply/valid.md");
    let mut bytes = b"\xEF\xBB\xBF".to_vec();
    bytes.extend(std::fs::read(source).unwrap());
    std::fs::write(dir.path().join(REPLY_FILE), bytes).unwrap();
    let envelope = read_reply(dir.path(), AgentRole::Reviewer, &declared()).unwrap();
    assert_eq!(envelope.status, "completed");
}
