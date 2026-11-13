//! The PR page verbs of `aidp gh` and `aidp sonar report --pr` against a fake `gh`.

use crate::common;

use aidp_testkit::{ProcShim, TempWorkspace};
use common::*;
use serde_json::Value;
use std::fs;
use std::process::Output;

const GH: &str = "gh";
const HELP_FLAG: &str = "--help";
const RULES_FILE: &str = "gh.rules";
const PATH_VAR: &str = "PATH";
const REPO_SLUG: &str = "owner/name";
const HEAD_SHA: &str = "abc123";
const REVIEW_BODY_FILE: &str = "review-body.md";
const REVIEW_BODY: &str = "Looks good overall.";
const COMMENTS_FILE: &str = "drafts.json";
const NO_DRAFTS: &str = "[]";
const VERBS: [&str; 11] = [
    "pr-detail",
    "pr-files",
    "pr-file-diff",
    "comment-create",
    "comment-edit",
    "comment-delete",
    "review-comment-create",
    "review-comment-edit",
    "review-comment-delete",
    "review-submit",
    "pr-status",
];
const FILES_PAYLOAD: &str =
    include_str!("../../aidp-integrations/src/tests/units/fixtures/pr_detail/files.json");
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

/// The help of `gh` lists every PR page verb.
#[test]
fn gh_help_lists_the_pr_page_verbs() {
    let ws = workspace();
    let output = aidp(&ws)
        .args([GH, HELP_FLAG])
        .output()
        .expect("run aidp gh --help");
    assert_eq!(output.status.code(), Some(0));
    let help = String::from_utf8_lossy(&output.stdout);
    for verb in VERBS {
        assert!(help.contains(verb), "help should list {verb}: {help}");
    }
}

/// One page of files comes back with its paging fields and the requested path.
#[test]
fn pr_files_prints_one_page_of_files() {
    let ws = workspace();
    let mut gh = FakeGh::new(&ws);
    gh.rule("pulls/7/files", 0, FILES_PAYLOAD);
    let output = gh.run(
        &ws,
        &[
            "pr-files",
            REPO_SLUG,
            "7",
            "--page",
            "2",
            "--per-page",
            "10",
        ],
    );
    assert_eq!(output.status.code(), Some(0));
    let page = stdout_json(&output);
    assert_eq!(page["page"], 2);
    assert_eq!(page["perPage"], 10);
    assert!(page["files"]
        .as_array()
        .is_some_and(|files| !files.is_empty()));
}

/// A gh failure on a read exits 5.
#[test]
fn pr_files_exits_five_when_gh_fails() {
    let ws = workspace();
    let mut gh = FakeGh::new(&ws);
    gh.rule("api", 1, "");
    let output = gh.run(&ws, &["pr-files", REPO_SLUG, "7"]);
    assert_eq!(output.status.code(), Some(5));
}

/// The review is one POST to the reviews endpoint with the request body from a file.
#[test]
fn review_submit_posts_one_request_to_the_reviews_endpoint() {
    let ws = workspace();
    let shim = ProcShim::new(GH, &ws);
    let body = ws.path().join(REVIEW_BODY_FILE);
    let drafts = ws.path().join(COMMENTS_FILE);
    fs::write(&body, REVIEW_BODY).expect("write body");
    fs::write(&drafts, NO_DRAFTS).expect("write drafts");
    let _ = aidp(&ws)
        .args([
            GH,
            "review-submit",
            REPO_SLUG,
            "7",
            "--event",
            "APPROVE",
            "--head-sha",
            HEAD_SHA,
            "--body-file",
        ])
        .arg(&body)
        .arg("--comments-file")
        .arg(&drafts)
        .output()
        .expect("run aidp gh review-submit");
    let argv = shim.argv();
    assert!(
        argv.iter()
            .any(|arg| arg == "repos/owner/name/pulls/7/reviews"),
        "reviews endpoint expected: {argv:?}"
    );
    assert!(
        argv.iter().any(|arg| arg == "POST"),
        "POST expected: {argv:?}"
    );
    assert!(
        argv.iter().any(|arg| arg == "--input"),
        "body file expected: {argv:?}"
    );
}

/// An unknown review event is a usage fault.
#[test]
fn review_submit_rejects_an_unknown_event() {
    let ws = workspace();
    let output = aidp(&ws)
        .args([
            GH,
            "review-submit",
            REPO_SLUG,
            "7",
            "--event",
            "MERGE",
            "--body-file",
            "-",
            "--comments-file",
            "none.json",
        ])
        .output()
        .expect("run aidp gh review-submit");
    assert_eq!(output.status.code(), Some(7));
}
