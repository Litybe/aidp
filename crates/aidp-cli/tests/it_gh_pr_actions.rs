//! `aidp gh` pull request action verbs against a scripted fake `gh`.

use crate::common;
use aidp_testkit::TempWorkspace;
use common::gh_shim::{install_gh_shim, CALLS_FILE, RULES_FILE};
use common::*;
use serde::Serialize;
use serde_json::Value;
use std::fs;
use std::path::Path;
use std::process::Output;

const GH: &str = "gh";
const PATH_VAR: &str = "PATH";
const REPO_FLAG: &str = "--repo";
const REPO_SLUG: &str = "owner/name";
const MERGED_PAYLOAD: &str =
    include_str!("../../aidp-integrations/src/tests/units/fixtures/pr/graphql_merged.json");
const NO_DECISION_PAYLOAD: &str = include_str!(
    "../../aidp-integrations/src/tests/units/fixtures/pr/graphql_open_no_decision.json"
);
const RULES_ONE_APPROVAL_PAYLOAD: &str =
    include_str!("../../aidp-integrations/src/tests/units/fixtures/pr/rules_none.json");
const NEGATIVE_VERDICT_EXIT: i32 = 6;
const GATE_CLOSED_EXIT: i32 = 4;
const NEEDLE_GRAPHQL: &str = "graphql";
const NEEDLE_RULES: &str = "rules/branches";
const NEEDLE_COMPARE: &str = "compare";
const NEEDLE_MERGE_CALL: &str = "pr merge";
const COMPARE_UP_TO_DATE: &str = r#"{"ahead_by":1,"behind_by":0,"status":"ahead"}"#;
const ROLLUP_CONTEXTS: &str = r#"[{"__typename":"CheckRun","name":"build","status":"COMPLETED","conclusion":"SUCCESS","detailsUrl":"u","startedAt":"2026-09-28T09:00:00Z"}]"#;
const RESOLVED_THREADS: &str = r#"[{"isResolved":true}]"#;
const APPROVED_ITEM_ID: &str = "approved";
const RULES_DEGRADED_ID: &str = "rules";
const NEEDLE_REPO: &str = "repos/owner/name";
const REPO_SQUASH_ALLOWED: &str = r#"{"allow_squash_merge":true}"#;
const GATE_KEY: &str = "gate";
const OK_KEY: &str = "ok";
const ITEMS_KEY: &str = "items";
const ID_KEY: &str = "id";
const REASON_KEY: &str = "reason";
const CHANGED_KEY: &str = "changed";
const DEGRADED_KEY: &str = "degraded";

/// The `data` envelope `gh api graphql` wraps around a payload.
#[derive(Serialize)]
struct GraphqlEnvelope {
    data: Value,
}
const HEAD_SHA: &str = "abc123";
const PR_NUMBER: &str = "123";
const EXPECTED_HEAD_FLAG: &str = "--expected-head-sha";
const REFUSED_EXIT: i32 = 1;

struct FakeGh {
    dir: std::path::PathBuf,
    rules: Vec<String>,
}

impl FakeGh {
    fn new(ws: &TempWorkspace) -> Self {
        let dir = install_gh_shim(ws.path(), "");
        Self {
            dir,
            rules: Vec::new(),
        }
    }

    fn rule(&mut self, needle: &str, exit_code: i32, stdout: &str) {
        let stdout_path = self.dir.join(format!("out{}.txt", self.rules.len()));
        fs::write(&stdout_path, stdout).expect("write canned stdout");
        self.rules
            .push(format!("{needle}\t{exit_code}\t{}", stdout_path.display()));
        fs::write(self.dir.join(RULES_FILE), self.rules.join("\n")).expect("write rules");
    }

    fn calls(&self) -> Vec<String> {
        fs::read_to_string(self.dir.join(CALLS_FILE))
            .unwrap_or_default()
            .lines()
            .map(str::to_string)
            .collect()
    }

    fn run(&self, ws: &TempWorkspace, args: &[&str]) -> Output {
        let existing = std::env::var_os(PATH_VAR).unwrap_or_default();
        let mut paths = vec![self.dir.clone()];
        paths.extend(std::env::split_paths(&existing));
        let joined = std::env::join_paths(paths).expect("join PATH");
        aidp(ws)
            .env(PATH_VAR, joined)
            .arg(GH)
            .args(args)
            .output()
            .expect("run aidp gh")
    }
}

fn stdout_json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("stdout is JSON")
}

fn green_no_decision_payload() -> String {
    let mut payload: Value = serde_json::from_str(NO_DECISION_PAYLOAD).expect("fixture is JSON");
    let pull_request = &mut payload["repository"]["pullRequest"];
    pull_request["commits"]["nodes"][0]["commit"]["statusCheckRollup"]["contexts"]["nodes"] =
        serde_json::from_str(ROLLUP_CONTEXTS).expect("contexts JSON");
    pull_request["reviewThreads"]["nodes"] =
        serde_json::from_str(RESOLVED_THREADS).expect("threads JSON");
    serde_json::to_string(&GraphqlEnvelope { data: payload }).expect("envelope serializes")
}

fn gh_with_unreadable_rules(ws: &TempWorkspace) -> FakeGh {
    let mut gh = FakeGh::new(ws);
    gh.rule(NEEDLE_RULES, REFUSED_EXIT, "");
    gh.rule(NEEDLE_COMPARE, 0, COMPARE_UP_TO_DATE);
    gh.rule(NEEDLE_GRAPHQL, 0, &green_no_decision_payload());
    gh.rule(NEEDLE_REPO, 0, REPO_SQUASH_ALLOWED);
    gh
}

fn merge_args() -> [&'static str; 6] {
    [
        "pr-merge",
        PR_NUMBER,
        EXPECTED_HEAD_FLAG,
        HEAD_SHA,
        REPO_FLAG,
        REPO_SLUG,
    ]
}

fn is_file(path: &Path) -> bool {
    path.is_file()
}

/// A refused thread is reported, the others still run, and the exit is 6.
#[test]
fn thread_resolve_reports_each_thread_and_exits_six_on_a_refusal() {
    let ws = workspace();
    let mut gh = FakeGh::new(&ws);
    gh.rule("id=B", 1, "");
    let output = gh.run(&ws, &["thread-resolve", "A", "B", "C"]);
    assert_eq!(output.status.code(), Some(NEGATIVE_VERDICT_EXIT));
    let results = stdout_json(&output);
    let results = results.as_array().expect("array of results");
    assert_eq!(results.len(), 3);
    assert_eq!(results[0][OK_KEY], true);
    assert_eq!(results[1][ID_KEY], "B");
    assert_eq!(results[1][OK_KEY], false);
    assert_eq!(results[2][OK_KEY], true);
    assert_eq!(gh.calls().len(), 3);
}

/// A closed gate refuses the merge before any `gh pr merge` call.
#[test]
fn pr_merge_with_a_closed_gate_never_calls_gh_pr_merge() {
    let ws = workspace();
    let mut gh = FakeGh::new(&ws);
    gh.rule("graphql", 0, MERGED_PAYLOAD);
    let output = gh.run(
        &ws,
        &[
            "pr-merge",
            "1",
            "--expected-head-sha",
            "abc123",
            REPO_FLAG,
            REPO_SLUG,
        ],
    );
    assert_ne!(output.status.code(), Some(0));
    assert!(
        !gh.calls().iter().any(|call| call.contains("pr merge")),
        "calls: {:?}",
        gh.calls()
    );
}

/// `--draft` reaches `gh pr create`.
#[test]
fn pr_create_draft_passes_the_draft_flag_to_gh() {
    let ws = workspace();
    let mut gh = FakeGh::new(&ws);
    gh.rule("pr create", 0, "https://github.com/owner/name/pull/1\n");
    let output = gh.run(
        &ws,
        &["pr-create", "--title", "t", "--body", "b", "--draft"],
    );
    assert_eq!(output.status.code(), Some(0));
    let create = gh
        .calls()
        .into_iter()
        .find(|call| call.contains("pr create"))
        .expect("pr create call");
    assert!(create.contains("--draft"), "call: {create}");
}

/// A PR already ready is left untouched.
#[test]
fn pr_ready_on_a_ready_pr_reports_no_change() {
    let ws = workspace();
    let mut gh = FakeGh::new(&ws);
    gh.rule("pr view", 0, r#"{"isDraft":false,"state":"OPEN"}"#);
    let output = gh.run(&ws, &["pr-ready", "1", REPO_FLAG, REPO_SLUG]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(stdout_json(&output)[CHANGED_KEY], false);
    assert!(!gh.calls().iter().any(|call| call.contains("pr ready")));
    assert!(is_file(&gh.dir.join(CALLS_FILE)));
}

/// No decision and an unreadable rules endpoint: the approval item passes with its reason.
#[test]
fn pr_status_without_an_approval_rule_passes_the_approved_item() {
    let ws = workspace();
    let gh = gh_with_unreadable_rules(&ws);
    let output = gh.run(&ws, &["pr-status", PR_NUMBER, REPO_FLAG, REPO_SLUG]);
    assert_eq!(output.status.code(), Some(0));
    let status = stdout_json(&output);
    assert_eq!(status[GATE_KEY][OK_KEY], true);
    let items = status[GATE_KEY][ITEMS_KEY].as_array().expect("gate items");
    let approved = items
        .iter()
        .find(|item| item[ID_KEY] == APPROVED_ITEM_ID)
        .expect("approved item");
    assert_eq!(approved[OK_KEY], true);
    assert_eq!(
        approved[REASON_KEY],
        aidp_integrations::pr::reason::NO_APPROVAL_RULE
    );
    let degraded = status[DEGRADED_KEY].as_array().expect("degraded list");
    assert!(degraded.iter().any(|entry| entry == RULES_DEGRADED_ID));
}

/// The same repository merges: `gh pr merge` is called and the exit is 0.
#[test]
fn pr_merge_without_an_approval_rule_calls_gh_pr_merge() {
    let ws = workspace();
    let gh = gh_with_unreadable_rules(&ws);
    let output = gh.run(&ws, &merge_args());
    assert_eq!(output.status.code(), Some(0));
    assert!(
        gh.calls()
            .iter()
            .any(|call| call.contains(NEEDLE_MERGE_CALL)),
        "calls: {:?}",
        gh.calls()
    );
}

/// A rule asking for one approval with none given closes the gate with exit 4.
#[test]
fn pr_merge_with_a_required_approval_missing_exits_four_without_merging() {
    let ws = workspace();
    let mut gh = FakeGh::new(&ws);
    gh.rule(NEEDLE_RULES, 0, RULES_ONE_APPROVAL_PAYLOAD);
    gh.rule(NEEDLE_COMPARE, 0, COMPARE_UP_TO_DATE);
    gh.rule(NEEDLE_GRAPHQL, 0, &green_no_decision_payload());
    let output = gh.run(&ws, &merge_args());
    assert_eq!(output.status.code(), Some(GATE_CLOSED_EXIT));
    assert!(
        !gh.calls()
            .iter()
            .any(|call| call.contains(NEEDLE_MERGE_CALL)),
        "calls: {:?}",
        gh.calls()
    );
}
