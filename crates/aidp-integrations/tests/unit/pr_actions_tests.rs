use super::*;
use serde_json::json;
use std::ffi::OsString;
use std::fs;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use tempfile::TempDir;

const VIEW_DRAFT: &str = include_str!("../../src/tests/units/fixtures/pr_actions_view.json");
const REPO_ALLOWS_SQUASH: &str =
    include_str!("../../src/tests/units/fixtures/pr_actions_repo.json");
const REPO_DENIES_SQUASH: &str = r#"{"allow_squash_merge":false,"allow_merge_commit":true}"#;
const GRAPHQL_OPEN: &str =
    include_str!("../../src/tests/units/fixtures/pr/graphql_open_pending.json");
const RULES_NONE: &str = include_str!("../../src/tests/units/fixtures/pr/rules_none.json");

const FAKE_DIR_VAR: &str = "FAKE_GH_DIR";
const PATH_VAR: &str = "PATH";
const CALLS_LOG: &str = "calls.log";
const REPO: &str = "o/r";
const PR_NUMBER: u64 = 123;
const HEAD_SHA: &str = "abc123";
const HEAD_BRANCH: &str = "feat/X-1";
const REBASED_SHA: &str = "def456";
const CHECKS_PATH: &str =
    "/repository/pullRequest/commits/nodes/0/commit/statusCheckRollup/contexts/nodes";
const THREADS_PATH: &str = "/repository/pullRequest/reviewThreads/nodes";

const NEEDLE_COMPARE: &str = "/compare/";
const NEEDLE_RULES: &str = "/rules/branches/";
const NEEDLE_GRAPHQL: &str = "graphql";
const NEEDLE_UPDATE: &str = "update-branch";
const NEEDLE_VIEW: &str = "pr view";
const NEEDLE_READY: &str = "pr ready";
const NEEDLE_MERGE: &str = "pr merge";
const NEEDLE_DELETE: &str = "DELETE";
const NEEDLE_REPO: &str = "api repos/o/r";
const MERGEABLE_VALUE: &str = "\"MERGEABLE\"";
const CONFLICTING_VALUE: &str = "\"CONFLICTING\"";
const REBASE_FLAG: &str = "--rebase";
const UPDATE_CONFLICT_ERROR: &str =
    "GraphQL: merge conflict between base and head (updatePullRequestBranch)";

#[cfg(windows)]
const SCRIPT_NAME: &str = "gh.cmd";
#[cfg(not(windows))]
const SCRIPT_NAME: &str = "gh";

#[cfg(windows)]
const SCRIPT: &str = "@echo off\r\n\
echo %* >>\"%FAKE_GH_DIR%\\calls.log\"\r\n\
echo %* >\"%FAKE_GH_DIR%\\last.args\"\r\n\
for %%R in (\"%FAKE_GH_DIR%\\rule_*.match\") do findstr /L /G:\"%%~fR\" \"%FAKE_GH_DIR%\\last.args\" >nul && set \"HIT=%%~dpnR\" && goto :found\r\n\
echo no rule matches 1>&2\r\n\
exit /b 99\r\n\
:found\r\n\
if exist \"%HIT%.out\" type \"%HIT%.out\"\r\n\
if exist \"%HIT%.err\" type \"%HIT%.err\" 1>&2\r\n\
set /p CODE=<\"%HIT%.code\"\r\n\
exit /b %CODE%\r\n";

#[cfg(not(windows))]
const SCRIPT: &str = "#!/bin/sh\n\
echo \"$*\" >> \"$FAKE_GH_DIR/calls.log\"\n\
echo \"$*\" > \"$FAKE_GH_DIR/last.args\"\n\
for match in \"$FAKE_GH_DIR\"/rule_*.match; do\n\
  if grep -qF -f \"$match\" \"$FAKE_GH_DIR/last.args\"; then\n\
    base=\"${match%.match}\"\n\
    [ -f \"$base.out\" ] && cat \"$base.out\"\n\
    [ -f \"$base.err\" ] && cat \"$base.err\" >&2\n\
    exit \"$(cat \"$base.code\")\"\n\
  fi\n\
done\n\
echo \"no rule matches\" >&2\n\
exit 99\n";

static ENV_LOCK: Mutex<()> = Mutex::new(());

struct Rule {
    needle: &'static str,
    stdout: String,
    stderr: &'static str,
    code: i32,
}

fn answer(needle: &'static str, stdout: &str) -> Rule {
    Rule {
        needle,
        stdout: stdout.to_string(),
        stderr: "",
        code: 0,
    }
}

fn failure(needle: &'static str, stderr: &'static str) -> Rule {
    Rule {
        needle,
        stdout: String::new(),
        stderr,
        code: 1,
    }
}

/// A `gh` shim first on PATH that logs argv and replays the first matching rule.
struct FakeGh {
    dir: TempDir,
    previous_path: OsString,
    _guard: MutexGuard<'static, ()>,
}

impl FakeGh {
    fn install(rules: &[Rule]) -> Self {
        let guard = ENV_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let dir = tempfile::tempdir().unwrap();
        for (index, rule) in rules.iter().enumerate() {
            write_rule(dir.path(), index + 1, rule);
        }
        write_script(dir.path());
        let previous_path = std::env::var_os(PATH_VAR).unwrap_or_default();
        let mut entries = vec![dir.path().to_path_buf()];
        entries.extend(std::env::split_paths(&previous_path));
        std::env::set_var(PATH_VAR, std::env::join_paths(entries).unwrap());
        std::env::set_var(FAKE_DIR_VAR, dir.path());
        FakeGh {
            dir,
            previous_path,
            _guard: guard,
        }
    }

    fn calls(&self) -> Vec<String> {
        fs::read_to_string(self.dir.path().join(CALLS_LOG))
            .unwrap_or_default()
            .lines()
            .map(|line| line.trim().to_string())
            .collect()
    }

    fn called(&self, needle: &str) -> bool {
        self.calls().iter().any(|line| line.contains(needle))
    }
}

impl Drop for FakeGh {
    fn drop(&mut self) {
        std::env::set_var(PATH_VAR, &self.previous_path);
        std::env::remove_var(FAKE_DIR_VAR);
    }
}

fn write_rule(dir: &Path, index: usize, rule: &Rule) {
    let base = dir.join(format!("rule_{index:02}"));
    fs::write(base.with_extension("match"), rule.needle).unwrap();
    fs::write(base.with_extension("out"), &rule.stdout).unwrap();
    fs::write(base.with_extension("err"), rule.stderr).unwrap();
    fs::write(base.with_extension("code"), rule.code.to_string()).unwrap();
}

fn write_script(dir: &Path) {
    let path = dir.join(SCRIPT_NAME);
    fs::write(&path, SCRIPT).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }
}

fn view_with(state: &str, draft: bool) -> String {
    json!({"isDraft": draft, "state": state}).to_string()
}

fn pending_graphql() -> String {
    let data: Value = serde_json::from_str(GRAPHQL_OPEN).unwrap();
    json!({"data": data}).to_string()
}

fn mergeable_graphql() -> String {
    let mut data: Value = serde_json::from_str(GRAPHQL_OPEN).unwrap();
    let passing = json!([{"__typename": "CheckRun", "name": "build", "status": "COMPLETED",
        "conclusion": "SUCCESS", "detailsUrl": "u", "startedAt": "2026-09-28T09:00:00Z"}]);
    *data.pointer_mut(CHECKS_PATH).unwrap() = passing;
    *data.pointer_mut(THREADS_PATH).unwrap() = json!([]);
    json!({"data": data}).to_string()
}

fn compare_behind(behind_by: u64) -> String {
    json!({"ahead_by": 1, "behind_by": behind_by, "status": "ahead"}).to_string()
}

fn merge_rules(
    graphql: String,
    repo: &str,
    behind_by: u64,
    merge: Rule,
    delete: Rule,
) -> Vec<Rule> {
    vec![
        answer(NEEDLE_COMPARE, &compare_behind(behind_by)),
        answer(NEEDLE_RULES, RULES_NONE),
        answer(NEEDLE_GRAPHQL, &graphql),
        answer(NEEDLE_UPDATE, ""),
        answer(NEEDLE_VIEW, &json!({"headRefOid": REBASED_SHA}).to_string()),
        merge,
        delete,
        answer(NEEDLE_REPO, repo),
    ]
}

fn happy_rules() -> Vec<Rule> {
    merge_rules(
        mergeable_graphql(),
        REPO_ALLOWS_SQUASH,
        0,
        answer(NEEDLE_MERGE, ""),
        answer(NEEDLE_DELETE, ""),
    )
}

#[test]
fn ready_on_a_draft_marks_it_ready() {
    let fake = FakeGh::install(&[answer(NEEDLE_VIEW, VIEW_DRAFT), answer(NEEDLE_READY, "")]);
    let outcome = pr_ready(PR_NUMBER, Some(REPO), false).unwrap();
    assert_eq!(
        outcome,
        ReadyOutcome {
            changed: true,
            draft: false
        }
    );
    assert!(fake.called(NEEDLE_READY));
}

#[test]
fn ready_on_an_already_ready_pr_changes_nothing() {
    let fake = FakeGh::install(&[
        answer(NEEDLE_VIEW, &view_with("OPEN", false)),
        answer(NEEDLE_READY, ""),
    ]);
    let outcome = pr_ready(PR_NUMBER, Some(REPO), false).unwrap();
    assert_eq!(
        outcome,
        ReadyOutcome {
            changed: false,
            draft: false
        }
    );
    assert!(!fake.called(NEEDLE_READY));
}

#[test]
fn ready_on_a_merged_pr_is_refused() {
    let fake = FakeGh::install(&[
        answer(NEEDLE_VIEW, &view_with("MERGED", false)),
        answer(NEEDLE_READY, ""),
    ]);
    let error = pr_ready(PR_NUMBER, Some(REPO), false).unwrap_err();
    assert!(matches!(error, IntegrationError::Conflict(_)));
    assert!(!fake.called(NEEDLE_READY));
}

#[test]
fn merge_is_refused_with_the_gate_reason_when_the_gate_is_closed() {
    let fake = FakeGh::install(&merge_rules(
        pending_graphql(),
        REPO_ALLOWS_SQUASH,
        0,
        answer(NEEDLE_MERGE, ""),
        answer(NEEDLE_DELETE, ""),
    ));
    let error = pr_merge(PR_NUMBER, Some(REPO), HEAD_SHA, false).unwrap_err();
    let IntegrationError::Conflict(reason) = error else {
        panic!("expected a conflict");
    };
    assert!(!reason.is_empty());
    assert!(!fake.called(NEEDLE_MERGE));
    assert!(!fake.called(NEEDLE_UPDATE));
}

#[test]
fn merge_stops_when_squash_is_disabled_and_tries_no_other_strategy() {
    let fake = FakeGh::install(&merge_rules(
        mergeable_graphql(),
        REPO_DENIES_SQUASH,
        0,
        answer(NEEDLE_MERGE, ""),
        answer(NEEDLE_DELETE, ""),
    ));
    let error = pr_merge(PR_NUMBER, Some(REPO), HEAD_SHA, false).unwrap_err();
    assert_eq!(error.to_string(), "squash disabled on this repository");
    assert!(!fake.called(NEEDLE_MERGE));
    assert!(!fake.called("--rebase"));
}

const FAST_POLL: MergeablePoll = MergeablePoll {
    timeout: Duration::from_millis(30),
    interval: Duration::from_millis(5),
};

#[test]
fn merge_on_a_behind_branch_rebases_then_really_merges_on_the_rebased_head() {
    let fake = FakeGh::install(&merge_rules(
        mergeable_graphql(),
        REPO_ALLOWS_SQUASH,
        2,
        answer(NEEDLE_MERGE, ""),
        answer(NEEDLE_DELETE, ""),
    ));
    let outcome = pr_merge(PR_NUMBER, Some(REPO), HEAD_SHA, false).unwrap();
    assert!(outcome.merged);
    assert!(outcome.rebased);
    assert_eq!(outcome.sha.as_deref(), Some(REBASED_SHA));
    let calls = fake.calls();
    let update = calls
        .iter()
        .position(|line| line.contains(NEEDLE_UPDATE) && line.contains("--rebase"))
        .unwrap();
    let merge = calls
        .iter()
        .position(|line| line.contains(NEEDLE_MERGE))
        .unwrap();
    assert!(update < merge);
    assert!(calls[merge].contains(REBASED_SHA));
    assert!(!calls[merge].contains(HEAD_SHA));
}

#[test]
fn a_rebase_conflict_updates_the_branch_by_merging_the_base() {
    let fake = FakeGh::install(&[
        failure(
            "--rebase",
            "GraphQL: rebase conflict between base and head (updatePullRequestBranch)",
        ),
        answer(NEEDLE_UPDATE, ""),
        answer(NEEDLE_VIEW, &json!({"headRefOid": REBASED_SHA}).to_string()),
    ]);
    let updated = pr_update_branch(PR_NUMBER, Some(REPO)).unwrap();
    assert_eq!(updated.head_sha.as_deref(), Some(REBASED_SHA));
    let updates: Vec<String> = fake
        .calls()
        .into_iter()
        .filter(|line| line.contains(NEEDLE_UPDATE))
        .collect();
    assert_eq!(updates.len(), 2, "{updates:?}");
    assert!(!updates[1].contains("--rebase"), "{updates:?}");
}

#[test]
fn merge_after_a_rebase_fails_when_github_never_computes_mergeable() {
    let unknown = mergeable_graphql().replace("\"MERGEABLE\"", "\"UNKNOWN\"");
    let fake = FakeGh::install(&merge_rules(
        unknown,
        REPO_ALLOWS_SQUASH,
        2,
        answer(NEEDLE_MERGE, ""),
        answer(NEEDLE_DELETE, ""),
    ));
    let error = pr_merge_polling(PR_NUMBER, Some(REPO), HEAD_SHA, false, FAST_POLL).unwrap_err();
    assert!(error.to_string().contains("mergeable"), "{error}");
    assert!(fake.called(NEEDLE_UPDATE));
    assert!(!fake.called(NEEDLE_MERGE));
}

fn head(sha: &str) -> Option<String> {
    Some(sha.to_string())
}

#[test]
fn waiting_for_the_rebase_ignores_the_old_head_even_when_it_looks_mergeable() {
    let mut answers = vec![
        (head(HEAD_SHA), Mergeable::Mergeable),
        (head(HEAD_SHA), Mergeable::Unknown),
        (head(REBASED_SHA), Mergeable::Unknown),
        (head(REBASED_SHA), Mergeable::Mergeable),
    ];
    let mut reads = 0;
    let result = wait_for_rebased_head(FAST_POLL, HEAD_SHA, || {
        reads += 1;
        Ok(answers.remove(0))
    });
    assert_eq!(result.unwrap(), REBASED_SHA);
    assert_eq!(reads, 4);
}

#[test]
fn waiting_for_the_rebase_reports_a_conflict_without_polling_further() {
    let mut reads = 0;
    let result = wait_for_rebased_head(FAST_POLL, HEAD_SHA, || {
        reads += 1;
        Ok((head(REBASED_SHA), Mergeable::Conflicting))
    });
    assert!(matches!(result, Err(IntegrationError::BaseConflict(_))));
    assert_eq!(reads, 1);
}

#[test]
fn waiting_for_the_rebase_fails_when_the_head_never_moves() {
    let error = wait_for_rebased_head(FAST_POLL, HEAD_SHA, || {
        Ok((head(HEAD_SHA), Mergeable::Mergeable))
    })
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "github did not apply the rebase within the allowed time"
    );
}

#[test]
fn merge_reports_a_moved_head() {
    let fake = FakeGh::install(&merge_rules(
        mergeable_graphql(),
        REPO_ALLOWS_SQUASH,
        0,
        failure(
            NEEDLE_MERGE,
            "GraphQL: Head branch was modified. Review and try the merge again. (mergePullRequest)",
        ),
        answer(NEEDLE_DELETE, ""),
    ));
    let error = pr_merge(PR_NUMBER, Some(REPO), HEAD_SHA, false).unwrap_err();
    assert_eq!(error.to_string(), "the branch moved since you read it");
    assert!(!fake.called(NEEDLE_DELETE));
}

#[test]
fn merge_squashes_pins_the_head_and_deletes_through_the_api() {
    let fake = FakeGh::install(&happy_rules());
    let outcome = pr_merge(PR_NUMBER, Some(REPO), HEAD_SHA, false).unwrap();
    assert!(outcome.merged);
    assert!(!outcome.rebased);
    assert!(outcome.branch_deleted);
    assert_eq!(outcome.sha.as_deref(), Some(HEAD_SHA));
    let calls = fake.calls();
    let merge_call = calls
        .iter()
        .find(|line| line.contains(NEEDLE_MERGE))
        .unwrap();
    assert!(merge_call.contains("--squash"));
    assert!(merge_call.contains("--match-head-commit"));
    assert!(merge_call.contains(HEAD_SHA));
    assert!(!merge_call.contains("--delete-branch"));
    assert!(calls.iter().any(|line| {
        line.contains(NEEDLE_DELETE) && line.contains(&format!("refs/heads/{HEAD_BRANCH}"))
    }));
}

#[test]
fn merge_keeps_the_branch_when_asked() {
    let fake = FakeGh::install(&happy_rules());
    let outcome = pr_merge(PR_NUMBER, Some(REPO), HEAD_SHA, true).unwrap();
    assert!(outcome.merged);
    assert!(!outcome.branch_deleted);
    assert!(!fake.called(NEEDLE_DELETE));
}

#[test]
fn merge_survives_a_refused_branch_deletion() {
    let _fake = FakeGh::install(&merge_rules(
        mergeable_graphql(),
        REPO_ALLOWS_SQUASH,
        0,
        answer(NEEDLE_MERGE, ""),
        failure(NEEDLE_DELETE, "Reference cannot be deleted"),
    ));
    let outcome = pr_merge(PR_NUMBER, Some(REPO), HEAD_SHA, false).unwrap();
    assert!(outcome.merged);
    assert!(!outcome.branch_deleted);
    assert_eq!(outcome.detail.len(), 1);
    assert!(outcome.detail[0].contains("Reference cannot be deleted"));
}

#[test]
fn thread_resolve_records_a_failure_and_still_runs_the_rest() {
    let fake = FakeGh::install(&[
        failure("id=T_B", "thread not found"),
        answer(NEEDLE_GRAPHQL, "{}"),
    ]);
    let ids = ["T_A", "T_B", "T_C"].map(String::from);
    let results = thread_resolve(&ids, false);
    let summary: Vec<(&str, bool)> = results
        .iter()
        .map(|entry| (entry.id.as_str(), entry.ok))
        .collect();
    assert_eq!(summary, [("T_A", true), ("T_B", false), ("T_C", true)]);
    assert!(results[1]
        .error
        .as_deref()
        .unwrap()
        .contains("thread not found"));
    assert_eq!(results[0].error, None);
    let calls = fake.calls();
    assert_eq!(calls.len(), 3);
    for (call, id) in calls.iter().zip(["id=T_A", "id=T_B", "id=T_C"]) {
        assert!(call.contains(id), "{call}");
        assert!(call.contains("resolveReviewThread"), "{call}");
    }
}

#[test]
fn a_gate_closed_by_a_conflict_alone_is_a_base_conflict() {
    let fake = FakeGh::install(&merge_rules(
        mergeable_graphql().replace(MERGEABLE_VALUE, CONFLICTING_VALUE),
        REPO_ALLOWS_SQUASH,
        0,
        answer(NEEDLE_MERGE, ""),
        answer(NEEDLE_DELETE, ""),
    ));
    let error = pr_merge(PR_NUMBER, Some(REPO), HEAD_SHA, false).unwrap_err();
    assert!(
        matches!(error, IntegrationError::BaseConflict(_)),
        "{error:?}"
    );
    assert!(!fake.called(NEEDLE_MERGE));
}

#[test]
fn a_gate_closed_by_a_conflict_and_red_checks_stays_a_plain_conflict() {
    let fake = FakeGh::install(&merge_rules(
        pending_graphql().replace(MERGEABLE_VALUE, CONFLICTING_VALUE),
        REPO_ALLOWS_SQUASH,
        0,
        answer(NEEDLE_MERGE, ""),
        answer(NEEDLE_DELETE, ""),
    ));
    let error = pr_merge(PR_NUMBER, Some(REPO), HEAD_SHA, false).unwrap_err();
    assert!(matches!(error, IntegrationError::Conflict(_)), "{error:?}");
    assert!(!fake.called(NEEDLE_MERGE));
}

#[test]
fn reporting_merge_turns_a_base_conflict_into_an_unmerged_outcome() {
    let fake = FakeGh::install(&merge_rules(
        mergeable_graphql().replace(MERGEABLE_VALUE, CONFLICTING_VALUE),
        REPO_ALLOWS_SQUASH,
        0,
        answer(NEEDLE_MERGE, ""),
        answer(NEEDLE_DELETE, ""),
    ));
    let outcome = pr_merge_reporting_conflict(PR_NUMBER, Some(REPO), HEAD_SHA, false).unwrap();
    assert!(!outcome.merged);
    assert!(outcome.conflict);
    assert_eq!(outcome.detail.len(), 1, "{outcome:?}");
    assert!(!fake.called(NEEDLE_MERGE));
}

#[test]
fn reporting_merge_keeps_other_refusals_as_errors() {
    let _fake = FakeGh::install(&merge_rules(
        pending_graphql(),
        REPO_ALLOWS_SQUASH,
        0,
        answer(NEEDLE_MERGE, ""),
        answer(NEEDLE_DELETE, ""),
    ));
    let error = pr_merge_reporting_conflict(PR_NUMBER, Some(REPO), HEAD_SHA, false).unwrap_err();
    assert!(matches!(error, IntegrationError::Conflict(_)), "{error:?}");
}

#[test]
fn a_behind_branch_whose_update_conflicts_is_a_base_conflict() {
    let mut rules = vec![
        failure(REBASE_FLAG, UPDATE_CONFLICT_ERROR),
        failure(NEEDLE_UPDATE, UPDATE_CONFLICT_ERROR),
    ];
    rules.extend(merge_rules(
        mergeable_graphql(),
        REPO_ALLOWS_SQUASH,
        2,
        answer(NEEDLE_MERGE, ""),
        answer(NEEDLE_DELETE, ""),
    ));
    let fake = FakeGh::install(&rules);
    let error = pr_merge(PR_NUMBER, Some(REPO), HEAD_SHA, false).unwrap_err();
    assert!(
        matches!(error, IntegrationError::BaseConflict(_)),
        "{error:?}"
    );
    assert!(!fake.called(NEEDLE_MERGE));
}
