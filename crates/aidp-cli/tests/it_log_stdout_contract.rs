use crate::common;
use aidp_testkit::TempWorkspace;
use assert_cmd::Command;
use common::*;
use std::path::Path;

const TRACE_LEVEL: &str = "trace";
const RUST_LOG_VAR: &str = "RUST_LOG";
const VERBOSE_FLAG: &str = "-vvv";
const TARGET_ID: &str = "T-LOG";
const PIPELINE_TEMPLATE: &str = "templates/dev-aidp.yaml";
const PIPELINE_INPUTS: &str = r#"{"prompt":"p","feature":"filet","track":"quick"}"#;
const VALIDATE_ARG: &str = "validate";
const DEV_TEMPLATE_ID: &str = "dev";
const CURRENT_DIR: &str = ".";
const PHASE_KEY: &str = "phase";
const AWAITING_PLAN_PHASE: &str = "awaiting_plan";
const RUN_ID_PLACEHOLDER: &str = "{run}";

/// How verbosity is raised for a child process.
#[derive(Clone, Copy, Debug)]
enum Verbosity {
    RustLogTrace,
    AidpLogTrace,
    VerboseFlag,
}

const ALL_VERBOSITIES: [Verbosity; 3] = [
    Verbosity::RustLogTrace,
    Verbosity::AidpLogTrace,
    Verbosity::VerboseFlag,
];

/// JSON-stdout commands; `{run}` is replaced by a real run id.
const JSON_COMMANDS: [&[&str]; 5] = [
    &[VALIDATE_ARG, DEV_TEMPLATE_ID],
    &["pipeline", "list", FORMAT_FLAG, FORMAT_JSON],
    &["vault", "status", FORMAT_FLAG, FORMAT_JSON],
    &[
        RUN_ARG,
        STATE_ARG,
        RUN_FLAG,
        RUN_ID_PLACEHOLDER,
        FORMAT_FLAG,
        FORMAT_JSON,
    ],
    &[
        WORKFLOW_ARG,
        PLAN_STATUS_ARG,
        RUN_FLAG,
        RUN_ID_PLACEHOLDER,
        FORMAT_FLAG,
        FORMAT_JSON,
    ],
];

/// Builds a command with real skills and the requested verbosity applied.
fn verbose_command(ws: &TempWorkspace, verbosity: Verbosity) -> Command {
    let mut cmd = aidp_with_fixture_skill(ws);
    match verbosity {
        Verbosity::RustLogTrace => {
            cmd.env(RUST_LOG_VAR, TRACE_LEVEL);
        }
        Verbosity::AidpLogTrace => {
            cmd.env(AIDP_LOG_VAR, TRACE_LEVEL);
        }
        Verbosity::VerboseFlag => {
            cmd.arg(VERBOSE_FLAG);
        }
    }
    cmd
}

/// Runs `run init` on the dev-aidp template and returns the raw stdout.
fn init_run(ws: &TempWorkspace, verbosity: Verbosity) -> std::process::Output {
    let mut cmd = verbose_command(ws, verbosity);
    cmd.args([RUN_ARG, INIT_ARG, PIPELINE_FLAG])
        .arg(skill_root().join(PIPELINE_TEMPLATE))
        .args([
            TARGET_FLAG,
            TARGET_ID,
            WORKSPACE_FLAG,
            CURRENT_DIR,
            INPUTS_FLAG,
            PIPELINE_INPUTS,
        ]);
    cmd.output().expect("run init should execute")
}

fn stdout_lines(output: &std::process::Output) -> Vec<String> {
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_string)
        .collect()
}

fn emit(
    ws: &TempWorkspace,
    verbosity: Verbosity,
    run_id: &str,
    out: &Path,
) -> std::process::Output {
    let mut cmd = verbose_command(ws, verbosity);
    cmd.args([
        WORKFLOW_ARG,
        EMIT_ARG,
        RUN_FLAG,
        run_id,
        WORKSPACE_FLAG,
        CURRENT_DIR,
        OUT_FLAG,
    ])
    .arg(out);
    cmd.output().expect("workflow emit should execute")
}

#[test]
fn json_stdout_commands_parse_whole_stdout_under_every_verbosity() {
    for verbosity in ALL_VERBOSITIES {
        let ws = workspace();
        let init = init_run(&ws, verbosity);
        assert!(init.status.success(), "{verbosity:?}: run init failed");
        let run_id = stdout_lines(&init).remove(0);
        for template in JSON_COMMANDS {
            let args: Vec<String> = template
                .iter()
                .map(|arg| arg.replace(RUN_ID_PLACEHOLDER, &run_id))
                .collect();
            let mut cmd = verbose_command(&ws, verbosity);
            cmd.args(&args).args([WORKSPACE_FLAG, CURRENT_DIR]);
            let output = cmd.output().expect("command should execute");
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(
                serde_json::from_str::<serde_json::Value>(&stdout).is_ok(),
                "{verbosity:?} {args:?}: stdout is not pure JSON: {stdout}"
            );
        }
    }
}

#[test]
fn run_init_prints_one_run_id_line_and_emit_prints_one_existing_path() {
    for verbosity in ALL_VERBOSITIES {
        let ws = workspace();
        let init = init_run(&ws, verbosity);
        assert!(init.status.success(), "{verbosity:?}: run init failed");
        let lines = stdout_lines(&init);
        assert_eq!(lines.len(), 1, "{verbosity:?}: init stdout: {lines:?}");
        assert!(
            is_run_id(&lines[0]),
            "{verbosity:?}: not a run id: {}",
            lines[0]
        );

        let mut cmd = verbose_command(&ws, verbosity);
        cmd.args([
            WORKFLOW_ARG,
            EMIT_ARG,
            RUN_FLAG,
            &lines[0],
            WORKSPACE_FLAG,
            CURRENT_DIR,
        ]);
        let output = cmd.output().expect("workflow emit should execute");
        assert!(output.status.success(), "{verbosity:?}: emit failed");
        let emitted = stdout_lines(&output);
        assert_eq!(emitted.len(), 1, "{verbosity:?}: emit stdout: {emitted:?}");
        assert!(
            ws.path().join(&emitted[0]).exists() || Path::new(&emitted[0]).exists(),
            "{verbosity:?}: emitted path missing: {}",
            emitted[0]
        );
    }
}

#[test]
fn plan_tasks_emit_keeps_phase_json_on_stderr_only() {
    for verbosity in ALL_VERBOSITIES {
        let ws = workspace();
        let out_dir = tempfile::TempDir::new().expect("tempdir");
        let init = init_run(&ws, verbosity);
        let run_id = stdout_lines(&init).remove(0);
        let output = emit(&ws, verbosity, &run_id, &out_dir.path().join("workflow.js"));
        assert!(output.status.success(), "{verbosity:?}: emit failed");

        let is_phase_line = |line: &str| {
            serde_json::from_str::<serde_json::Value>(line)
                .ok()
                .is_some_and(|value| value[PHASE_KEY] == AWAITING_PLAN_PHASE)
        };
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.lines().any(is_phase_line),
            "{verbosity:?}: no phase line on stderr: {stderr}"
        );
        assert!(
            !String::from_utf8_lossy(&output.stdout)
                .lines()
                .any(is_phase_line),
            "{verbosity:?}: phase line leaked to stdout"
        );
    }
}
