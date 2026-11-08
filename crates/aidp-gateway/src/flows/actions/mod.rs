//! The deterministic flow actions, each an `aidp` subprocess over one verb.

use super::registry::{ActionContext, FlowAction, Registry, SubprocessAction};
use crate::models::constants::flow_actions::*;
use crate::models::enums::BuiltinAction;
use aidp_flows::ActionOutcome;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

#[cfg(test)]
#[path = "../../../tests/unit/flow_actions_tests.rs"]
mod tests;

/// Turns a successful output carrying `skipped: true` into a skipped outcome.
pub fn skipped_when_flagged(outcome: ActionOutcome) -> ActionOutcome {
    let ActionOutcome::Ok(outputs) = &outcome else {
        return outcome;
    };
    if outputs.get(OUTPUT_SKIPPED).and_then(Value::as_bool) != Some(true) {
        return outcome;
    }
    let reason = outputs
        .get(OUTPUT_REASON)
        .and_then(Value::as_str)
        .unwrap_or(OUTPUT_SKIPPED)
        .to_string();
    ActionOutcome::Skipped { reason }
}

/// Turns a successful output carrying `waiting_external: true` into a waiting outcome.
pub fn waiting_when_flagged(outcome: ActionOutcome) -> ActionOutcome {
    let ActionOutcome::Ok(outputs) = &outcome else {
        return outcome;
    };
    if outputs
        .get(OUTPUT_WAITING_EXTERNAL)
        .and_then(Value::as_bool)
        != Some(true)
    {
        return outcome;
    }
    let reason = outputs
        .get(OUTPUT_REASON)
        .and_then(Value::as_str)
        .unwrap_or(OUTPUT_WAITING_EXTERNAL)
        .to_string();
    ActionOutcome::WaitingExternal {
        reason,
        outputs: outputs.clone(),
    }
}

/// A subprocess action whose `skipped: true` output marks the step skipped.
pub struct SkippableSubprocessAction {
    inner: SubprocessAction,
}

impl FlowAction for SkippableSubprocessAction {
    fn writes(&self) -> bool {
        self.inner.writes()
    }

    fn placeholders(&self) -> Vec<String> {
        self.inner.placeholders()
    }

    fn run(&self, context: &ActionContext<'_>, with: &BTreeMap<String, Value>) -> ActionOutcome {
        waiting_when_flagged(skipped_when_flagged(self.inner.run(context, with)))
    }
}

fn register_action(registry: &mut Registry, action: BuiltinAction, runner: SubprocessAction) {
    registry.register_action(
        action.as_str(),
        Arc::new(SkippableSubprocessAction { inner: runner }),
    );
}

/// `gh.issue_comment`: posts a report on the issue. `with.cost_footer: true` appends the ticket's
/// cost line (the flag is only passed then, so the placeholder list stays fixed).
pub struct IssueCommentAction;

impl IssueCommentAction {
    fn command(with: &BTreeMap<String, Value>) -> SubprocessAction {
        let mut args = vec![
            VERB_TICKET,
            VERB_TICKET_COMMENT,
            PLACEHOLDER_KEY_ARG,
            FLAG_BODY_FILE,
            PLACEHOLDER_BODY_FILE_ARG,
        ];
        if with.get(WITH_COST_FOOTER).and_then(Value::as_bool) == Some(true) {
            args.push(FLAG_COST_FOOTER);
        }
        SubprocessAction::new(args)
    }
}

impl FlowAction for IssueCommentAction {
    fn placeholders(&self) -> Vec<String> {
        vec![PLACEHOLDER_BODY_FILE_ARG
            .trim_matches(|c| c == '{' || c == '}')
            .to_string()]
    }

    fn run(&self, context: &ActionContext<'_>, with: &BTreeMap<String, Value>) -> ActionOutcome {
        let mut with = with.clone();
        let temporary = materialize_body(&mut with);
        let outcome = Self::command(&with).run(context, &with);
        if let Some(path) = temporary {
            let _ = std::fs::remove_file(path);
        }
        outcome
    }
}

/// `--body-file` reads a file, but `with.body` is the comment text of a template (a report file
/// path stays as it is): text that is no existing file goes to a temporary file whose path
/// replaces it. Returns that file, to delete after the call.
fn materialize_body(with: &mut BTreeMap<String, Value>) -> Option<PathBuf> {
    let text = with.get(WITH_BODY).and_then(Value::as_str)?.to_string();
    if Path::new(&text).is_file() {
        return None;
    }
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "aidp-comment-{}-{}.md",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, &text).ok()?;
    with.insert(
        WITH_BODY.to_string(),
        Value::String(path.to_string_lossy().to_string()),
    );
    Some(path)
}

/// Registers the flow actions. The writers hold the ticket workspace.
pub fn register(registry: &mut Registry) {
    register_action(
        registry,
        BuiltinAction::GitWorktree,
        SubprocessAction::new([
            VERB_GATEWAY,
            VERB_GATEWAY_WORKTREES,
            VERB_WORKTREES_ENSURE,
            PLACEHOLDER_KEY_ARG,
        ])
        .holding_workspace(),
    );
    register_action(
        registry,
        BuiltinAction::GitBranch,
        SubprocessAction::new([VERB_FLOW, VERB_FLOW_GIT_BRANCH, PLACEHOLDER_KEY_ARG])
            .holding_workspace(),
    );
    // The main checkout of the project is the workspace of these two (workspace: project): the
    // executor serializes them with the `unity-editor` resource, not with a worktree lock.
    register_action(
        registry,
        BuiltinAction::GitCheckout,
        SubprocessAction::new([VERB_FLOW, VERB_FLOW_GIT_CHECKOUT, PLACEHOLDER_KEY_ARG]),
    );
    register_action(
        registry,
        BuiltinAction::GitRestore,
        SubprocessAction::new([VERB_FLOW, VERB_FLOW_GIT_RESTORE, PLACEHOLDER_KEY_ARG]),
    );
    register_action(
        registry,
        BuiltinAction::GitPush,
        SubprocessAction::new([
            VERB_FLOW,
            VERB_FLOW_GIT_PUSH,
            FLAG_BRANCH,
            PLACEHOLDER_BRANCH_ARG,
        ])
        .timeout(Duration::from_secs(GIT_PUSH_TIMEOUT_SECONDS))
        .holding_workspace(),
    );
    register_action(
        registry,
        BuiltinAction::GhPrCreate,
        SubprocessAction::new([
            VERB_FLOW,
            VERB_FLOW_PR_CREATE,
            PLACEHOLDER_KEY_ARG,
            FLAG_BRANCH,
            PLACEHOLDER_BRANCH_ARG,
            FLAG_BODY_FILE,
            PLACEHOLDER_PR_BODY_ARG,
            FLAG_DRAFT,
        ]),
    );
    register_action(
        registry,
        BuiltinAction::GhPrReady,
        SubprocessAction::new([VERB_GH, VERB_GH_PR_READY, PLACEHOLDER_PR_ARG]),
    );
    register_action(
        registry,
        BuiltinAction::GhMerge,
        SubprocessAction::new([
            VERB_GH,
            VERB_GH_PR_MERGE,
            PLACEHOLDER_PR_ARG,
            FLAG_EXPECTED_HEAD_SHA,
            PLACEHOLDER_SHA_ARG,
        ]),
    );
    register_action(
        registry,
        BuiltinAction::GhIssueLabel,
        SubprocessAction::new([
            VERB_TICKET,
            VERB_TICKET_LABEL,
            PLACEHOLDER_KEY_ARG,
            PLACEHOLDER_STATE_ARG,
        ]),
    );
    registry.register_action(
        BuiltinAction::GhIssueComment.as_str(),
        Arc::new(IssueCommentAction),
    );
    register_action(
        registry,
        BuiltinAction::WorktreeRemove,
        SubprocessAction::new([
            VERB_GATEWAY,
            VERB_GATEWAY_WORKTREES,
            VERB_WORKTREES_REMOVE,
            PLACEHOLDER_KEY_ARG,
        ])
        .holding_workspace(),
    );
}
