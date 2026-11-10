use super::register;
use crate::flows::registry::{ActionContext, FlowAction, Registry, SubprocessAction};
use crate::models::constants::flow_actions::*;
use crate::models::enums::BuiltinAction;
use aidp_core::process_runner::{ProcessRunner, ProcessSpec, SpawnedChild};
use aidp_flows::ActionOutcome;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::{Cursor, Read, Write};
use std::sync::{Arc, Mutex};

const TICKET: &str = "PIXEL-12";
const PR_NUMBER: u64 = 42;
const EXIT_OK: i32 = 0;
const EXIT_BUSINESS: i32 = 3;
const EXIT_TECHNICAL: i32 = 1;
const CHANGED_FALSE_OUTPUT: &str = "{\"changed\":false}";
const WITH_PR: &str = "pr";
const WITH_SHA: &str = "sha";
const SHA: &str = "abc123";
const CHANGED_KEY: &str = "changed";
const WITH_BRANCH: &str = "branch";
const WITH_PR_BODY: &str = "pr_body";
const BRANCH: &str = "feat/PIXEL-12";
const PR_BODY_PATH: &str = "C:/vault/Pipelines/svc/PIXEL-12/run/commit_pr/pr-body.md";

/// Replays one canned child and records every spec it was asked to spawn.
struct RecordingRunner {
    code: i32,
    stdout: String,
    specs: Mutex<Vec<ProcessSpec>>,
}

impl RecordingRunner {
    fn new(code: i32, stdout: &str) -> Self {
        RecordingRunner {
            code,
            stdout: stdout.to_string(),
            specs: Mutex::new(Vec::new()),
        }
    }

    fn spawned_args(&self) -> Vec<String> {
        self.specs.lock().unwrap()[0].args.clone()
    }

    fn spawn_count(&self) -> usize {
        self.specs.lock().unwrap().len()
    }
}

struct CannedChild {
    code: i32,
    stdout: Option<Cursor<Vec<u8>>>,
}

impl SpawnedChild for CannedChild {
    fn take_stdout(&mut self) -> Option<Box<dyn Read + Send>> {
        self.stdout
            .take()
            .map(|cursor| Box::new(cursor) as Box<dyn Read + Send>)
    }

    fn take_stdin(&mut self) -> Option<Box<dyn Write + Send>> {
        None
    }

    fn id(&self) -> Option<u32> {
        None
    }

    fn wait(&mut self) -> std::io::Result<i32> {
        Ok(self.code)
    }

    fn try_wait(&mut self) -> std::io::Result<Option<i32>> {
        Ok(Some(self.code))
    }

    fn kill(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl ProcessRunner for RecordingRunner {
    fn output(&self, _spec: &ProcessSpec) -> std::io::Result<std::process::Output> {
        Err(std::io::Error::other("output is not used by actions"))
    }

    fn spawn(&self, spec: &ProcessSpec) -> std::io::Result<Box<dyn SpawnedChild>> {
        self.specs.lock().unwrap().push(spec.clone());
        Ok(Box::new(CannedChild {
            code: self.code,
            stdout: Some(Cursor::new(self.stdout.clone().into_bytes())),
        }))
    }
}

fn run_with(
    action: &dyn FlowAction,
    runner: &RecordingRunner,
    key: &str,
    with: BTreeMap<String, Value>,
) -> ActionOutcome {
    let context = ActionContext {
        key,
        workspace: None,
        runner,
    };
    action.run(&context, &with)
}

fn registered(name: BuiltinAction) -> Arc<dyn FlowAction> {
    Registry::with_builtins()
        .action(name.as_str())
        .expect("action is registered")
}

fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| value.to_string()).collect()
}

#[test]
fn templating_fills_key_and_with_values() {
    let action = SubprocessAction::new([
        VERB_GH,
        VERB_GH_PR_READY,
        PLACEHOLDER_PR_ARG,
        PLACEHOLDER_KEY_ARG,
    ]);
    let with = BTreeMap::from([(WITH_PR.to_string(), json!(PR_NUMBER))]);
    let args = action.resolve_args(TICKET, &with).unwrap();
    assert_eq!(args, strings(&[VERB_GH, VERB_GH_PR_READY, "42", TICKET]));
}

#[test]
fn templating_reports_the_missing_value() {
    let action = SubprocessAction::new([VERB_GH, PLACEHOLDER_PR_ARG]);
    let error = action.resolve_args(TICKET, &BTreeMap::new()).unwrap_err();
    assert!(error.contains("`pr`"));
}

#[test]
fn missing_value_is_a_technical_error_and_spawns_nothing() {
    let runner = RecordingRunner::new(EXIT_OK, "");
    let outcome = run_with(
        registered(BuiltinAction::GhPrReady).as_ref(),
        &runner,
        TICKET,
        BTreeMap::new(),
    );
    assert!(matches!(outcome, ActionOutcome::TechnicalError(_)));
    assert_eq!(runner.spawn_count(), 0);
}

#[test]
fn every_builtin_action_is_registered() {
    let registry = Registry::with_builtins();
    for action in BuiltinAction::ALL {
        assert!(
            registry.action(action.as_str()).is_some(),
            "{}",
            action.as_str()
        );
    }
}

#[test]
fn only_the_workspace_writers_hold_the_workspace() {
    let writers = [
        BuiltinAction::GitWorktree,
        BuiltinAction::GitBranch,
        BuiltinAction::GitPush,
        BuiltinAction::WorktreeRemove,
    ];
    for action in BuiltinAction::ALL {
        let expected = writers.contains(&action);
        assert_eq!(registered(action).writes(), expected, "{}", action.as_str());
    }
}

/// The publish runs on the flow's branch, read from the run's facts like every placeholder.
#[test]
fn git_push_publishes_the_flow_branch() {
    let runner = RecordingRunner::new(EXIT_OK, "");
    let action = registered(BuiltinAction::GitPush);
    assert_eq!(action.placeholders(), strings(&[WITH_BRANCH]));
    let with = BTreeMap::from([(WITH_BRANCH.to_string(), json!(BRANCH))]);
    let outcome = run_with(action.as_ref(), &runner, TICKET, with);
    assert!(matches!(outcome, ActionOutcome::Ok(_)));
    assert_eq!(
        runner.spawned_args()[..4],
        strings(&[VERB_FLOW, VERB_FLOW_GIT_PUSH, FLAG_BRANCH, BRANCH])[..]
    );
}

/// The pull request opens as a draft on the flow's branch, with the body file a pipeline wrote.
#[test]
fn pr_create_opens_a_draft_from_the_pipeline_body() {
    let runner = RecordingRunner::new(EXIT_OK, "");
    let action = registered(BuiltinAction::GhPrCreate);
    assert_eq!(action.placeholders(), strings(&[WITH_BRANCH, WITH_PR_BODY]));
    let with = BTreeMap::from([
        (WITH_BRANCH.to_string(), json!(BRANCH)),
        (WITH_PR_BODY.to_string(), json!(PR_BODY_PATH)),
    ]);
    run_with(action.as_ref(), &runner, TICKET, with);
    assert_eq!(
        runner.spawned_args()[..8],
        strings(&[
            VERB_FLOW,
            VERB_FLOW_PR_CREATE,
            TICKET,
            FLAG_BRANCH,
            BRANCH,
            FLAG_BODY_FILE,
            PR_BODY_PATH,
            FLAG_DRAFT
        ])[..]
    );
}

#[test]
fn merge_pins_the_head_sha() {
    let runner = RecordingRunner::new(EXIT_OK, r#"{"merged":true}"#);
    let with = BTreeMap::from([
        (WITH_PR.to_string(), json!(PR_NUMBER)),
        (WITH_SHA.to_string(), json!(SHA)),
    ]);
    let outcome = run_with(
        registered(BuiltinAction::GhMerge).as_ref(),
        &runner,
        TICKET,
        with,
    );
    assert!(matches!(outcome, ActionOutcome::Ok(_)));
    let args = runner.spawned_args();
    assert_eq!(
        args[..6],
        strings(&[
            VERB_GH,
            VERB_GH_PR_MERGE,
            "42",
            FLAG_EXPECTED_HEAD_SHA,
            SHA,
            FLAG_REPORT_CONFLICT
        ])[..]
    );
}

#[test]
fn merge_blocked_by_a_base_conflict_ends_the_step_with_the_conflict_output() {
    let runner = RecordingRunner::new(
        EXIT_OK,
        r#"{"merged":false,"conflict":true,"detail":["this PR has merge conflicts"]}"#,
    );
    let with = BTreeMap::from([
        (WITH_PR.to_string(), json!(PR_NUMBER)),
        (WITH_SHA.to_string(), json!(SHA)),
    ]);
    let outcome = run_with(
        registered(BuiltinAction::GhMerge).as_ref(),
        &runner,
        TICKET,
        with,
    );
    let ActionOutcome::Ok(outputs) = outcome else {
        panic!("expected a done step, got {outcome:?}");
    };
    assert_eq!(outputs.get(OUTPUT_CONFLICT), Some(&json!(true)));
    assert_eq!(outputs.get(OUTPUT_MERGED), Some(&json!(false)));
}

#[test]
fn merge_that_did_not_merge_fails_the_step_with_its_outputs() {
    let runner = RecordingRunner::new(
        EXIT_OK,
        r#"{"merged":false,"rebased":true,"branchDeleted":false}"#,
    );
    let with = BTreeMap::from([
        (WITH_PR.to_string(), json!(PR_NUMBER)),
        (WITH_SHA.to_string(), json!(SHA)),
    ]);
    let outcome = run_with(
        registered(BuiltinAction::GhMerge).as_ref(),
        &runner,
        TICKET,
        with,
    );
    let ActionOutcome::TechnicalError(message) = outcome else {
        panic!("expected a failed step, got {outcome:?}");
    };
    assert!(message.contains("without merging"), "{message}");
    assert!(message.contains("rebased"), "{message}");
}

#[test]
fn business_exit_is_business_and_other_exit_is_technical() {
    let with = BTreeMap::from([(WITH_PR.to_string(), json!(PR_NUMBER))]);
    let business = RecordingRunner::new(EXIT_BUSINESS, "gate is red");
    let outcome = run_with(
        registered(BuiltinAction::GhPrReady).as_ref(),
        &business,
        TICKET,
        with.clone(),
    );
    assert!(matches!(outcome, ActionOutcome::BusinessError(_)));
    let technical = RecordingRunner::new(EXIT_TECHNICAL, "boom");
    let outcome = run_with(
        registered(BuiltinAction::GhPrReady).as_ref(),
        &technical,
        TICKET,
        with,
    );
    assert!(matches!(outcome, ActionOutcome::TechnicalError(_)));
}

#[test]
fn changed_false_passes_through_as_an_output() {
    let runner = RecordingRunner::new(EXIT_OK, CHANGED_FALSE_OUTPUT);
    let with = BTreeMap::from([(WITH_PR.to_string(), json!(PR_NUMBER))]);
    let outcome = run_with(
        registered(BuiltinAction::GhPrReady).as_ref(),
        &runner,
        TICKET,
        with,
    );
    let ActionOutcome::Ok(outputs) = outcome else {
        panic!("expected a successful outcome");
    };
    assert_eq!(outputs.get(CHANGED_KEY), Some(&json!(false)));
}

#[test]
fn register_adds_every_action_to_a_bare_registry() {
    let mut registry = Registry::default();
    register(&mut registry);
    let names = registry.catalog(None).actions;
    assert_eq!(names.len(), BuiltinAction::ALL.len() - 2);
}

#[test]
fn issue_label_replaces_the_state_label_through_the_ticket_verb() {
    let runner = RecordingRunner::new(EXIT_OK, "{\"changed\":true}");
    let with = BTreeMap::from([("state".to_string(), json!("review"))]);
    let outcome = run_with(
        registered(BuiltinAction::GhIssueLabel).as_ref(),
        &runner,
        "GH-12",
        with,
    );
    assert!(matches!(outcome, ActionOutcome::Ok(_)));
    let args = runner.spawned_args();
    let tail = &args[args.len() - 6..];
    assert_eq!(
        tail,
        strings(&[
            "ticket",
            "label",
            "GH-12",
            "review",
            "--format",
            "json-compact"
        ])
        .as_slice()
    );
}

#[test]
fn issue_label_without_a_state_is_a_technical_error() {
    let runner = RecordingRunner::new(EXIT_OK, "");
    let outcome = run_with(
        registered(BuiltinAction::GhIssueLabel).as_ref(),
        &runner,
        "GH-12",
        BTreeMap::new(),
    );
    assert!(matches!(outcome, ActionOutcome::TechnicalError(_)));
    assert_eq!(runner.spawn_count(), 0);
}

#[test]
fn issue_comment_posts_the_report_file_and_only_asks_for_the_footer_when_told() {
    let plain = RecordingRunner::new(EXIT_OK, "{}");
    let dir = tempfile::tempdir().unwrap();
    let report = dir.path().join("report.md");
    std::fs::write(&report, "rapport").unwrap();
    let report = report.to_string_lossy().to_string();
    let with = BTreeMap::from([("body".to_string(), json!(report))]);
    run_with(
        registered(BuiltinAction::GhIssueComment).as_ref(),
        &plain,
        "GH-12",
        with.clone(),
    );
    let args = plain.spawned_args();
    assert!(args.windows(5).any(|w| w
        == strings(&["ticket", "comment", "GH-12", "--body-file", report.as_str()]).as_slice()));
    assert!(!args.contains(&"--cost-footer".to_string()));

    let footer = RecordingRunner::new(EXIT_OK, "{}");
    let mut with_footer = with;
    with_footer.insert("cost_footer".to_string(), json!(true));
    run_with(
        registered(BuiltinAction::GhIssueComment).as_ref(),
        &footer,
        "GH-12",
        with_footer,
    );
    assert!(footer.spawned_args().contains(&"--cost-footer".to_string()));
}

#[test]
fn issue_comment_writes_a_text_body_to_a_file_for_body_file() {
    let runner = RecordingRunner::new(EXIT_OK, "{}");
    let with = BTreeMap::from([("body".to_string(), json!("Revue : done"))]);
    run_with(
        registered(BuiltinAction::GhIssueComment).as_ref(),
        &runner,
        "GH-12",
        with,
    );
    let args = runner.spawned_args();
    let at = args.iter().position(|a| a == "--body-file").unwrap();
    assert_ne!(args[at + 1], "Revue : done");
    assert!(args[at + 1].contains("aidp-comment-"));
}

/// The refresh runs the flow verb alone: the checkout it works in is the action's workspace.
#[test]
fn unity_refresh_runs_the_flow_verb_without_placeholders() {
    let runner = RecordingRunner::new(EXIT_OK, "{\"ok\":true,\"blocked\":false}");
    let action = registered(BuiltinAction::UnityRefresh);
    assert!(action.placeholders().is_empty());
    let outcome = run_with(action.as_ref(), &runner, TICKET, BTreeMap::new());
    let ActionOutcome::Ok(outputs) = outcome else {
        panic!("expected a successful outcome");
    };
    assert_eq!(outputs.get("ok"), Some(&json!(true)));
    let args = runner.spawned_args();
    let verb = strings(&[VERB_FLOW, VERB_FLOW_UNITY_REFRESH]);
    assert!(
        args.windows(verb.len())
            .any(|window| window == verb.as_slice()),
        "{args:?}"
    );
}

/// A closed editor is no failure of the refresh: the step is skipped and the test stage decides.
#[test]
fn unity_refresh_skipped_when_the_verb_says_so() {
    let runner = RecordingRunner::new(
        EXIT_OK,
        "{\"skipped\":true,\"reason\":\"unity-editor-closed\"}",
    );
    let outcome = run_with(
        registered(BuiltinAction::UnityRefresh).as_ref(),
        &runner,
        TICKET,
        BTreeMap::new(),
    );
    assert!(
        matches!(outcome, ActionOutcome::Skipped { reason } if reason == "unity-editor-closed")
    );
}

#[test]
fn bug_create_sends_the_report_as_a_file_and_the_summary_whole() {
    let runner = RecordingRunner::new(EXIT_OK, "{\"bug\":\"GH-300\",\"capped\":false}");
    let action = registered(BuiltinAction::BugCreate);
    let with = BTreeMap::from([
        (WITH_REPORT.to_string(), json!("Le sprite {hero} manque")),
        (WITH_SUMMARY.to_string(), json!("  sprite {hero} absent ")),
    ]);
    let outcome = run_with(action.as_ref(), &runner, TICKET, with);
    let ActionOutcome::Ok(outputs) = outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(outputs["bug"], json!("GH-300"));
    let args = runner.spawned_args();
    let at = |name: &str| args.iter().position(|arg| arg == name).unwrap();
    assert_eq!(args[at(VERB_FLOW_BUG_CREATE) + 1], TICKET);
    let report_file = &args[at(FLAG_REPORT_FILE) + 1];
    assert!(report_file.contains("aidp-bug-report"), "{report_file}");
    // Braces in the texts are not placeholders; the summary is trimmed and sent as one argument.
    assert!(args.contains(&format!("{FLAG_SUMMARY_PREFIX}sprite {{hero}} absent")));
    // The temporary report file is gone once the verb returned.
    assert!(!std::path::Path::new(report_file).exists());
}

#[test]
fn bug_create_leaves_out_an_empty_summary_and_an_absent_report() {
    let runner = RecordingRunner::new(EXIT_OK, "{}");
    let action = registered(BuiltinAction::BugCreate);
    let with = BTreeMap::from([(WITH_SUMMARY.to_string(), json!("  "))]);
    run_with(action.as_ref(), &runner, TICKET, with);
    let args = runner.spawned_args();
    assert!(!args.iter().any(|arg| arg.starts_with(FLAG_SUMMARY_PREFIX)));
    assert!(!args.iter().any(|arg| arg == FLAG_REPORT_FILE));
}

#[test]
fn bug_create_finds_a_report_named_without_a_path_under_the_run_folder() {
    let run_dir = tempfile::tempdir().unwrap();
    let stage = run_dir.path().join("stages").join("test");
    std::fs::create_dir_all(&stage).unwrap();
    let report = stage.join("test-report.md");
    std::fs::write(&report, "## Échec\nCA-1 ko").unwrap();
    let runner = RecordingRunner::new(EXIT_OK, "{}");
    let action = registered(BuiltinAction::BugCreate);
    let with = BTreeMap::from([
        (WITH_REPORT.to_string(), json!("test-report.md")),
        (
            WITH_RUN_DIR.to_string(),
            json!(run_dir.path().to_string_lossy()),
        ),
    ]);
    run_with(action.as_ref(), &runner, TICKET, with);
    let args = runner.spawned_args();
    let at = args.iter().position(|arg| arg == FLAG_REPORT_FILE).unwrap();
    assert_eq!(std::path::Path::new(&args[at + 1]), report.as_path());
    assert!(report.exists(), "the real report is never deleted");
}

#[test]
fn bug_create_sends_the_written_body_found_under_its_own_run_folder() {
    let test_dir = tempfile::tempdir().unwrap();
    let write_dir = tempfile::tempdir().unwrap();
    let stage = write_dir.path().join("stages").join("write");
    std::fs::create_dir_all(&stage).unwrap();
    let body = stage.join("bug.md");
    std::fs::write(&body, "## Issue de base\nrésumé").unwrap();
    let runner = RecordingRunner::new(EXIT_OK, "{}");
    let action = registered(BuiltinAction::BugCreate);
    let with = BTreeMap::from([
        (WITH_BODY.to_string(), json!("bug.md")),
        (
            WITH_BODY_DIR.to_string(),
            json!(write_dir.path().to_string_lossy()),
        ),
        (
            WITH_RUN_DIR.to_string(),
            json!(test_dir.path().to_string_lossy()),
        ),
    ]);
    run_with(action.as_ref(), &runner, TICKET, with);
    let args = runner.spawned_args();
    let at = args.iter().position(|arg| arg == FLAG_BODY_FILE).unwrap();
    assert_eq!(std::path::Path::new(&args[at + 1]), body.as_path());
    assert!(body.exists(), "the real body is never deleted");
}

#[test]
fn bug_create_falls_back_when_the_written_body_is_absent() {
    let run_dir = tempfile::tempdir().unwrap();
    for body in ["bug.md", "   ", ""] {
        let runner = RecordingRunner::new(EXIT_OK, "{}");
        let action = registered(BuiltinAction::BugCreate);
        let with = BTreeMap::from([
            (WITH_BODY.to_string(), json!(body)),
            (
                WITH_RUN_DIR.to_string(),
                json!(run_dir.path().to_string_lossy()),
            ),
        ]);
        run_with(action.as_ref(), &runner, TICKET, with);
        let args = runner.spawned_args();
        assert!(
            !args.iter().any(|arg| arg == FLAG_BODY_FILE),
            "{body:?}: {args:?}"
        );
    }
}

#[test]
fn bug_create_sends_a_written_body_given_as_text_through_a_temporary_file() {
    let runner = RecordingRunner::new(EXIT_OK, "{}");
    let action = registered(BuiltinAction::BugCreate);
    let with = BTreeMap::from([(WITH_BODY.to_string(), json!("## Bug\nligne {x}"))]);
    run_with(action.as_ref(), &runner, TICKET, with);
    let args = runner.spawned_args();
    let at = args.iter().position(|arg| arg == FLAG_BODY_FILE).unwrap();
    assert!(args[at + 1].contains("aidp-bug-body"), "{args:?}");
    assert!(!std::path::Path::new(&args[at + 1]).exists());
}

#[test]
fn issue_close_closes_the_issue_of_the_ticket() {
    let runner = RecordingRunner::new(EXIT_OK, "{\"changed\":true}");
    let action = registered(BuiltinAction::GhIssueClose);
    assert!(!action.writes());
    let outcome = run_with(action.as_ref(), &runner, TICKET, BTreeMap::new());
    assert!(matches!(outcome, ActionOutcome::Ok(_)));
    assert_eq!(
        runner.spawned_args()[..3],
        strings(&[VERB_TICKET, VERB_TICKET_CLOSE, TICKET])[..]
    );
}
