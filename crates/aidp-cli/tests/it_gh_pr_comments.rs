//! `aidp gh pr-comments` and the enriched `pr-review-threads --all` against a scripted fake `gh`.

use crate::common;
use aidp_testkit::TempWorkspace;
use common::*;
use serde_json::Value;
use std::fs;
use std::process::Output;

const GH: &str = "gh";
const RULES_FILE: &str = "gh.rules";
const PATH_VAR: &str = "PATH";
const GATEWAY_DIR_VAR: &str = "AIDP_GATEWAY_DIR";
const REPO_FLAG: &str = "--repo";
const REPO_SLUG: &str = "owner/name";
const STATE_SUBDIR: &str = "prs";
const STATE_FILE: &str = "owner__name__1.json";
const HANDED_THREAD: &str = "T_conv";
const CONVERSATION_PAYLOAD: &str =
    include_str!("../../aidp-integrations/tests/unit/fixtures/issue_comments.json");
const REVIEWS_PAYLOAD: &str =
    include_str!("../../aidp-integrations/tests/unit/fixtures/review_bodies.json");
const THREADS_PAYLOAD: &str =
    include_str!("../../aidp-integrations/tests/unit/fixtures/review_threads_conversation.json");
const PR_VIEW_PAYLOAD: &str = r#"{"number":1,"url":"https://github.com/owner/name/pull/1"}"#;
const SHIM_SOURCE: &str = r#"
fn main() {
    let dir = std::env::current_exe().unwrap().parent().unwrap().to_path_buf();
    let joined = std::env::args().skip(1).collect::<Vec<_>>().join(" ");
    let rules = std::fs::read_to_string(dir.join("gh.rules")).unwrap_or_default();
    for line in rules.lines() {
        let mut parts = line.splitn(3, '\t');
        let needle = parts.next().unwrap_or("");
        let code: i32 = parts.next().unwrap_or("0").parse().unwrap_or(0);
        let stdout_file = parts.next().unwrap_or("");
        if !needle.is_empty() && joined.contains(needle) {
            if !stdout_file.is_empty() {
                print!("{}", std::fs::read_to_string(stdout_file).unwrap_or_default());
            }
            if code != 0 {
                eprintln!("refused");
            }
            std::process::exit(code);
        }
    }
}
"#;

struct FakeGh {
    dir: std::path::PathBuf,
    rules: Vec<String>,
}

impl FakeGh {
    fn new(ws: &TempWorkspace) -> Self {
        let dir = ws.path().join("bin");
        fs::create_dir_all(&dir).expect("create shim dir");
        gh_shim::install_compiled_shim(&dir, SHIM_SOURCE);
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

    fn run(&self, ws: &TempWorkspace, gateway_dir: &std::path::Path, args: &[&str]) -> Output {
        let existing = std::env::var_os(PATH_VAR).unwrap_or_default();
        let mut paths = vec![self.dir.clone()];
        paths.extend(std::env::split_paths(&existing));
        let joined = std::env::join_paths(paths).expect("join PATH");
        aidp(ws)
            .env(PATH_VAR, joined)
            .env(GATEWAY_DIR_VAR, gateway_dir)
            .arg(GH)
            .args(args)
            .output()
            .expect("run aidp gh")
    }
}

fn stdout_json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).expect("stdout is JSON")
}

/// Points carry their bot flag and the summary comment is left out.
#[test]
fn pr_comments_prints_points_with_bot_flags_and_skips_the_summary() {
    let ws = workspace();
    let mut gh = FakeGh::new(&ws);
    gh.rule("issues/1/comments", 0, CONVERSATION_PAYLOAD);
    gh.rule("pulls/1/reviews", 0, REVIEWS_PAYLOAD);
    let output = gh.run(
        &ws,
        &ws.path().join("gateway"),
        &["pr-comments", "1", REPO_FLAG, REPO_SLUG],
    );
    assert_eq!(output.status.code(), Some(0));
    let comments = stdout_json(&output);
    let comments = comments.as_array().expect("array of comments");
    let ids: Vec<u64> = comments.iter().map(|c| c["id"].as_u64().unwrap()).collect();
    assert_eq!(ids, vec![11, 12, 13, 21]);
    let sonar = comments.iter().find(|c| c["id"] == 12).unwrap();
    assert_eq!(sonar["bot"], true);
    assert_eq!(sonar["points"].as_array().unwrap().len(), 2);
    let human = comments.iter().find(|c| c["id"] == 11).unwrap();
    assert_eq!(human["bot"], false);
    assert_eq!(human["points"][0]["id"], "11#0");
}

/// A gh failure exits 5.
#[test]
fn pr_comments_exits_five_when_gh_fails() {
    let ws = workspace();
    let mut gh = FakeGh::new(&ws);
    gh.rule("api", 1, "");
    let output = gh.run(
        &ws,
        &ws.path().join("gateway"),
        &["pr-comments", "1", REPO_FLAG, REPO_SLUG],
    );
    assert_eq!(output.status.code(), Some(5));
}

/// Threads carry `lastBy`, `waitingOnReviewer` and `addressed=indirect` for a handed id.
#[test]
fn review_threads_all_marks_handed_threads_indirect() {
    let ws = workspace();
    let gateway = ws.path().join("gateway");
    fs::create_dir_all(gateway.join(STATE_SUBDIR)).expect("create state dir");
    fs::write(
        gateway.join(STATE_SUBDIR).join(STATE_FILE),
        format!(r#"{{"handed":["{HANDED_THREAD}"]}}"#),
    )
    .expect("write state");
    let mut gh = FakeGh::new(&ws);
    gh.rule("graphql", 0, THREADS_PAYLOAD);
    gh.rule("pr view", 0, PR_VIEW_PAYLOAD);
    let output = gh.run(
        &ws,
        &gateway,
        &["pr-review-threads", "1", REPO_FLAG, REPO_SLUG, "--all"],
    );
    assert_eq!(output.status.code(), Some(0));
    let document = stdout_json(&output);
    let threads = document["threads"].as_array().expect("threads");
    assert_eq!(threads.len(), 3);
    let handed = threads
        .iter()
        .find(|t| t["thread_id"] == HANDED_THREAD)
        .unwrap();
    assert_eq!(handed["addressed"], "indirect");
    assert_eq!(handed["lastBy"], "me");
    assert_eq!(handed["startLine"], 8);
    assert!(handed["waitingOnReviewer"].is_boolean());
    let outdated = threads
        .iter()
        .find(|t| t["thread_id"] == "T_outdated")
        .unwrap();
    assert_eq!(outdated["addressed"], "direct");
    assert_eq!(outdated["lastBy"], "other");
}
