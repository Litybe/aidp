use super::*;
use aidp_core::TypedExit;

const HELP: &str = "--help";
const VERSION: &str = "--version";
const UNKNOWN: &str = "unknown-command";
const GATEWAY: &str = "gateway";
const SERVE: &str = "serve";
const PAYLOAD: &str = "payload";
const DIAGNOSTIC: &str = "diagnostic";
const FAILURE: &str = "refused";
const CHILD_CODE: i32 = 43;

fn arguments(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_string()).collect()
}

#[test]
fn help_and_version_are_successful_stdout_without_process_exit() {
    for flag in [HELP, VERSION] {
        let result = execute_embedded(arguments(&[flag]));
        assert_eq!(result.exit_code, i32::from(ExitStatus::Success.code()));
        assert!(!result.stdout.is_empty());
        assert!(result.stderr.is_empty());
    }
}

#[test]
fn invalid_arguments_are_usage_stderr_and_servers_are_policy_refusals() {
    let result = execute_embedded(arguments(&[UNKNOWN]));
    assert_eq!(result.exit_code, i32::from(ExitStatus::Usage.code()));
    assert!(result.stdout.is_empty());
    assert!(result.stderr.contains(UNKNOWN));
    let result = execute_embedded(arguments(&[GATEWAY, SERVE]));
    assert_eq!(
        result,
        command_error(ExitStatus::PolicyRefusal, SERVER_REFUSAL)
    );
}

#[test]
fn captured_output_keeps_diagnostics_failure_and_child_exit_semantics() {
    let output = CommandOutput::text(PAYLOAD)
        .with_diagnostics(vec![DIAGNOSTIC.to_string()])
        .with_failure(
            TypedExit {
                status: ExitStatus::PolicyRefusal,
                message: FAILURE.to_string(),
            }
            .into(),
        )
        .with_child_exit(CHILD_CODE);
    let result = finish_embedded(output);
    assert_eq!(result.exit_code, CHILD_CODE);
    assert_eq!(result.stdout, format!("{PAYLOAD}{LINE_ENDING}"));
    assert_eq!(
        result.stderr,
        format!("{DIAGNOSTIC}{LINE_ENDING}{ERROR_PREFIX}{FAILURE}{LINE_ENDING}")
    );
    let result = finish_embedded(
        CommandOutput::empty().with_failure(
            TypedExit {
                status: ExitStatus::PolicyRefusal,
                message: FAILURE.to_string(),
            }
            .into(),
        ),
    );
    assert_eq!(
        result.exit_code,
        i32::from(ExitStatus::PolicyRefusal.code())
    );
    assert!(result.stdout.is_empty());
}
