//! Every command a flow action spawns must parse with this CLI.
//!
//! The gateway tests only compare the arguments with its own constants, so a
//! drift between the two crates went unseen: `jira.transition` passed the key
//! positionally while `jira issue-transition` takes `--key`, and failed with exit 7.

use super::parse_cli_from;
use aidp_core::process_runner::{ProcessRunner, ProcessSpec, SpawnedChild};
use aidp_gateway::flows::registry::{ActionContext, BuiltinAction, FlowAction, Registry};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

const PROGRAM: &str = "aidp";
const TICKET: &str = "PIXEL-12";
const STATUS_KEY: &str = "in-progress";

/// Actions that spawn no command: `unity.refresh` waits on the editor through the wait extensions.
const NON_SPAWNING_ACTIONS: [BuiltinAction; 3] = [
    BuiltinAction::Noop,
    BuiltinAction::Fail,
    BuiltinAction::UnityRefresh,
];

/// Records the spawned arguments and reports a clean exit.
#[derive(Default)]
struct RecordingRunner {
    args: Mutex<Option<Vec<String>>>,
}

struct DoneChild;

impl SpawnedChild for DoneChild {
    fn take_stdout(&mut self) -> Option<Box<dyn Read + Send>> {
        None
    }

    fn take_stdin(&mut self) -> Option<Box<dyn Write + Send>> {
        None
    }

    fn id(&self) -> Option<u32> {
        None
    }

    fn wait(&mut self) -> std::io::Result<i32> {
        Ok(0)
    }

    fn try_wait(&mut self) -> std::io::Result<Option<i32>> {
        Ok(Some(0))
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
        *self.args.lock().unwrap() = Some(spec.args.clone());
        Ok(Box::new(DoneChild))
    }
}

/// A value for every placeholder an action can name, plus the transition target.
fn with_values() -> BTreeMap<String, Value> {
    BTreeMap::from([
        ("to".to_string(), json!(STATUS_KEY)),
        ("pr".to_string(), json!(42)),
        ("repo".to_string(), json!("acme/widgets")),
        ("sha".to_string(), json!("abc123")),
        (
            "url".to_string(),
            json!("https://github.com/acme/widgets/pull/42"),
        ),
        ("branch".to_string(), json!("feat/PIXEL-12")),
        ("body".to_string(), json!("report.md")),
        ("pr_body".to_string(), json!("pr-body.md")),
        ("state".to_string(), json!("review")),
    ])
}

/// The registered action.
fn action(builtin: BuiltinAction) -> Arc<dyn FlowAction> {
    Registry::with_builtins()
        .action(builtin.as_str())
        .expect("action is registered")
}

#[test]
fn every_subprocess_action_spawns_a_command_the_cli_accepts() {
    let mut checked = 0;
    for builtin in BuiltinAction::ALL {
        let runner = RecordingRunner::default();
        let context = ActionContext {
            key: TICKET,
            workspace: None,
            runner: &runner,
        };
        action(builtin).run(&context, &with_values());
        let Some(args) = runner.args.lock().unwrap().take() else {
            continue;
        };
        let argv = std::iter::once(PROGRAM.to_string()).chain(args.iter().cloned());
        if let Err(error) = parse_cli_from(argv) {
            panic!("{} spawns `{}`: {error}", builtin.as_str(), args.join(" "));
        }
        checked += 1;
    }
    assert_eq!(
        checked,
        BuiltinAction::ALL.len() - NON_SPAWNING_ACTIONS.len()
    );
}
