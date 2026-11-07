use super::*;
use std::path::PathBuf;

const RUN_ID: &str = "run-anchor";
const SIMPLE_RESULT_FILE: &str = "workflow-result.json";
const SPACED_RESULT_FILE: &str = "my run/workflow result.json";
const EXPECTED_SIMPLE: &str =
    "aidp run record-resume --run run-anchor --from-result workflow-result.json";
const EXPECTED_SPACED: &str =
    "aidp run record-resume --run run-anchor --from-result my run/workflow result.json";

/// The full command line is rendered with single spaces between tokens.
#[test]
fn record_resume_from_result_renders_the_full_command_line() {
    let command =
        AidpCommand::record_resume_from_result(RUN_ID, &PathBuf::from(SIMPLE_RESULT_FILE));
    assert_eq!(command.to_string(), EXPECTED_SIMPLE);
}

/// A path containing spaces is rendered verbatim, without quoting.
#[test]
fn record_resume_from_result_renders_a_path_with_spaces_verbatim() {
    let command =
        AidpCommand::record_resume_from_result(RUN_ID, &PathBuf::from(SPACED_RESULT_FILE));
    assert_eq!(command.to_string(), EXPECTED_SPACED);
}

const ACTION_ARGS: [&str; 2] = ["gh", "pr-create"];
const EXPECTED_JSON_ACTION: &str = "aidp gh pr-create --format json-compact";

/// A flow action appends the JSON output flag to its arguments.
#[test]
fn json_action_appends_the_json_format_flag() {
    let command = AidpCommand::json_action(ACTION_ARGS);
    assert_eq!(command.to_string(), EXPECTED_JSON_ACTION);
    assert_eq!(command.args().len(), 4);
}
