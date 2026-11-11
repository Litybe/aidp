//! The deterministic flow actions, each an `aidp` subprocess over one verb.

use super::registry::{ActionContext, FlowAction, Registry, SubprocessAction};
use super::unity_wait::UnityRefreshAction;
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

const NOT_MERGED_MESSAGE: &str = "gh.merge finished without merging the pull request";

/// A successful merge output must carry `merged: true` or `conflict: true` (the flow then resolves
/// the conflict and merges again); anything else fails the step.
pub fn merged_or_failed(outcome: ActionOutcome) -> ActionOutcome {
    let ActionOutcome::Ok(outputs) = &outcome else {
        return outcome;
    };
    let flagged = |key: &str| outputs.get(key).and_then(Value::as_bool) == Some(true);
    if flagged(OUTPUT_MERGED) || flagged(OUTPUT_CONFLICT) {
        return outcome;
    }
    let detail = Value::Object(outputs.clone().into_iter().collect());
    ActionOutcome::TechnicalError(format!("{NOT_MERGED_MESSAGE}: {detail}"))
}

/// `gh.merge`: done when the PR really is merged, or when it conflicts with its base (`conflict`).
pub struct MergeAction {
    inner: SubprocessAction,
}

impl FlowAction for MergeAction {
    fn writes(&self) -> bool {
        self.inner.writes()
    }

    fn placeholders(&self) -> Vec<String> {
        self.inner.placeholders()
    }

    fn run(&self, context: &ActionContext<'_>, with: &BTreeMap<String, Value>) -> ActionOutcome {
        merged_or_failed(self.inner.run(context, with))
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
    materialize_text(with, WITH_BODY, "aidp-comment")
}

/// [`materialize_body`] for the `with` key `key`; the temporary file name starts with `prefix`.
fn materialize_text(
    with: &mut BTreeMap<String, Value>,
    key: &str,
    prefix: &str,
) -> Option<PathBuf> {
    let text = with.get(key).and_then(Value::as_str)?.to_string();
    if Path::new(&text).is_file() {
        return None;
    }
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "{prefix}-{}-{}.md",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, &text).ok()?;
    with.insert(
        key.to_string(),
        Value::String(path.to_string_lossy().to_string()),
    );
    Some(path)
}

/// `bug.create`: raises the bug issue of a failed test (`aidp flow bug-create KEY`). `with.report`
/// is the failure report (text, or the path of a report file), `with.summary` the one-line summary
/// of the title. `with.body` is the description the `bug-write` stage wrote (a file name such as
/// `bug.md`, looked up under `with.body_dir` or else `with.run_dir`, or a path, or the text itself):
/// the daemon keeps the marker first and folds the report at the end. A body that is empty or
/// cannot be found is left out: the ticket is created anyway, with the report alone. Outputs `bug`, `number`, `url`, `parent`, `base`, `count`, `capped`, `changed`,
/// `linked`: `capped: true` means the parent already holds the most bugs and nothing was created.
pub struct BugCreateAction;

impl BugCreateAction {
    /// The verb line: texts travel as `{name}` placeholders, filled once, so braces inside them stay.
    fn command(with: &BTreeMap<String, Value>) -> SubprocessAction {
        let mut args = vec![
            VERB_FLOW.to_string(),
            VERB_FLOW_BUG_CREATE.to_string(),
            PLACEHOLDER_KEY_ARG.to_string(),
        ];
        if with.contains_key(WITH_REPORT) {
            args.push(FLAG_REPORT_FILE.to_string());
            args.push(format!("{{{WITH_REPORT}}}"));
        }
        let has_summary = with
            .get(WITH_SUMMARY)
            .and_then(Value::as_str)
            .is_some_and(|summary| !summary.trim().is_empty());
        if has_summary {
            args.push(format!("{FLAG_SUMMARY_PREFIX}{{{WITH_SUMMARY}}}"));
        }
        let allow_extra = with.get(WITH_ALLOW_EXTRA).and_then(Value::as_bool) == Some(true);
        if allow_extra {
            args.push(FLAG_ALLOW_EXTRA.to_string());
        }
        if with.contains_key(WITH_BODY) {
            args.push(FLAG_BODY_FILE.to_string());
            args.push(format!("{{{WITH_BODY}}}"));
        }
        SubprocessAction::new(args)
    }
}

/// Deepest folder level searched under the run folder for a report named without a path.
const REPORT_SEARCH_DEPTH: usize = 6;
/// A report value longer than this is text, never a file name.
const REPORT_NAME_MAX_CHARS: usize = 200;

/// The file called `name` under `root`, looking `depth` levels down.
fn find_named_file(root: &Path, name: &std::ffi::OsStr, depth: usize) -> Option<PathBuf> {
    let mut folders = Vec::new();
    for entry in std::fs::read_dir(root).ok()?.flatten() {
        let path = entry.path();
        if path.is_dir() {
            folders.push(path);
        } else if path.file_name() == Some(name) {
            return Some(path);
        }
    }
    if depth == 0 {
        return None;
    }
    folders
        .iter()
        .find_map(|folder| find_named_file(folder, name, depth - 1))
}

/// A stage names its file by name (`test-report.md`): with a run folder (`with.<dir_key>`, else
/// `with.run_dir`) the file is looked up under it and `with.<key>` becomes its path. Returns whether
/// the value was a file name (found or not). Text and existing paths stay as they are.
fn locate_named(with: &mut BTreeMap<String, Value>, key: &str, dir_key: &str) -> bool {
    let Some(value) = with.get(key).and_then(Value::as_str) else {
        return false;
    };
    let value = value.trim();
    if value.is_empty()
        || value.contains('\n')
        || value.chars().count() > REPORT_NAME_MAX_CHARS
        || Path::new(value).is_file()
    {
        return false;
    }
    let Some(name) = Path::new(value).file_name() else {
        return true;
    };
    let dir = [dir_key, WITH_RUN_DIR].into_iter().find_map(|candidate| {
        with.get(candidate)
            .and_then(Value::as_str)
            .map(Path::new)
            .filter(|dir| dir.is_dir())
    });
    if let Some(found) = dir.and_then(|dir| find_named_file(dir, name, REPORT_SEARCH_DEPTH)) {
        with.insert(
            key.to_string(),
            Value::String(found.to_string_lossy().to_string()),
        );
    }
    true
}

/// The report of the test, named by file name or given as text.
fn locate_report(with: &mut BTreeMap<String, Value>) {
    locate_named(with, WITH_REPORT, WITH_RUN_DIR);
}

/// The written body: a file name that no file answers to is dropped (the fallback body is used),
/// an empty value too. What remains is a path or the text itself.
fn locate_body(with: &mut BTreeMap<String, Value>) {
    let is_name = locate_named(with, WITH_BODY, WITH_BODY_DIR);
    let missing = match with.get(WITH_BODY).and_then(Value::as_str) {
        None => false,
        Some(value) => value.trim().is_empty() || (is_name && !Path::new(value).is_file()),
    };
    if missing {
        with.remove(WITH_BODY);
    }
}

impl FlowAction for BugCreateAction {
    fn run(&self, context: &ActionContext<'_>, with: &BTreeMap<String, Value>) -> ActionOutcome {
        let mut with = with.clone();
        locate_report(&mut with);
        locate_body(&mut with);
        if let Some(summary) = with.get(WITH_SUMMARY).and_then(Value::as_str) {
            let trimmed = Value::String(summary.trim().to_string());
            with.insert(WITH_SUMMARY.to_string(), trimmed);
        }
        let temporaries = [
            materialize_text(&mut with, WITH_REPORT, "aidp-bug-report"),
            materialize_text(&mut with, WITH_BODY, "aidp-bug-body"),
        ];
        let outcome = Self::command(&with).run(context, &with);
        for path in temporaries.into_iter().flatten() {
            let _ = std::fs::remove_file(path);
        }
        outcome
    }
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
    registry.register_action(
        BuiltinAction::GhMerge.as_str(),
        Arc::new(MergeAction {
            inner: SubprocessAction::new([
                VERB_GH,
                VERB_GH_PR_MERGE,
                PLACEHOLDER_PR_ARG,
                FLAG_EXPECTED_HEAD_SHA,
                PLACEHOLDER_SHA_ARG,
                FLAG_REPORT_CONFLICT,
            ]),
        }),
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
    register_action(
        registry,
        BuiltinAction::TicketToTest,
        SubprocessAction::new([
            VERB_TICKET,
            VERB_TICKET_LABEL,
            PLACEHOLDER_KEY_ARG,
            STATE_TO_TEST,
        ]),
    );
    registry.register_action(
        BuiltinAction::GhIssueComment.as_str(),
        Arc::new(IssueCommentAction),
    );
    registry.register_action(BuiltinAction::BugCreate.as_str(), Arc::new(BugCreateAction));
    // A bug merged into its parent's branch does not close its issue by itself.
    register_action(
        registry,
        BuiltinAction::GhIssueClose,
        SubprocessAction::new([VERB_TICKET, VERB_TICKET_CLOSE, PLACEHOLDER_KEY_ARG]),
    );
    // Runs in the project checkout (workspace: project) right after `git.checkout`: the editor's
    // reimport and recompilation of the new branch end before the test stage starts.
    let extensions = registry.wait_extensions();
    registry.register_action(
        BuiltinAction::UnityRefresh.as_str(),
        Arc::new(UnityRefreshAction::new(extensions)),
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
