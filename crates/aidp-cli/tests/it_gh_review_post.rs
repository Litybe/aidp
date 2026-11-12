//! `aidp gh review-post` and `thread-reopen` against a scripted fake `gh`.

use crate::common;
use aidp_testkit::TempWorkspace;
use common::*;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::Output;

const GH: &str = "gh";
const RULES_FILE: &str = "gh.rules";
const CALLS_FILE: &str = "gh.calls";
const INPUTS_FILE: &str = "gh.inputs";
const PATH_VAR: &str = "PATH";
const REPO_SLUG: &str = "owner/name";
const PR_NUMBER: &str = "7";
const RUN_ID: &str = "run-7";
const PASS: &str = "1";
const REVIEW_POST: &str = "review-post";
const THREAD_REOPEN: &str = "thread-reopen";
const FINDINGS_FLAG: &str = "--findings";
const PASS_FLAG: &str = "--pass";
const RUN_FLAG: &str = "--run";
const BODY_FILE_FLAG: &str = "--body-file";
const INPUT_NEEDLE: &str = "--input";
const REVIEWS_NEEDLE: &str = "/reviews";
const FILES_NEEDLE: &str = "/files?";
const PULL_NEEDLE: &str = "pulls/7";
const THREADS_NEEDLE: &str = "reviewThreads(";
const PR_VIEW_NEEDLE: &str = "pr view";
const REPLY_NEEDLE: &str = "addPullRequestReviewThreadReply";
const UNRESOLVE_NEEDLE: &str = "unresolveReviewThread";
const THREAD_ID: &str = "T-1";
const REPLY_BODY: &str = "still broken";
const FINDINGS_FILE: &str = "findings.json";
const REPLY_FILE: &str = "reply.md";
const FINDINGS: &str = include_str!("fixtures/review_post/findings.json");
const PR_FILES: &str = include_str!("fixtures/review_post/pr_files.json");
const NO_REVIEWS: &str = "[]";
const EMPTY_OBJECT: &str = "{}";
const PR_VIEW_REPLY: &str = r#"{"number":7,"headRefOid":"abc123"}"#;
const HEAD_REPLY: &str = r#"{"head":{"sha":"abc123"}}"#;
const SUBMIT_REPLY: &str = r#"{"id":99,"state":"COMMENTED"}"#;
const NO_THREADS: &str = r#"{"data":{"viewer":{"login":"bot"},"repository":{"pullRequest":{"reviewThreads":{"nodes":[]}}}}}"#;
const MARKED_THREADS: &str = r#"{"data":{"viewer":{"login":"bot"},"repository":{"pullRequest":{"reviewThreads":{"nodes":[
{"id":"T-1","isResolved":false,"path":"src/a.rs","line":11,"comments":{"totalCount":1,"nodes":[{"id":"C-1","body":"<!-- aidp:finding id=F-1 run=run-7 pass=1 -->\nx"}]}},
{"id":"T-2","isResolved":false,"path":"src/new_name.rs","line":2,"comments":{"totalCount":1,"nodes":[{"id":"C-2","body":"<!-- aidp:finding id=F-2 run=run-7 pass=1 -->\nx"}]}}
]}}}}}"#;
const MARKED_REVIEWS: &str = r#"[{"id":99,"body":"aidp review pass 1\n- <!-- aidp:finding id=F-3 run=run-7 pass=1 --> `src/a.rs:99` x"}]"#;
const SHIM_SOURCE: &str = r#"
use std::io::Write;

fn main() {
    let dir = std::env::current_exe().unwrap().parent().unwrap().to_path_buf();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let joined = args.join(" ");
    let mut calls = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("gh.calls"))
        .unwrap();
    writeln!(calls, "{joined}").unwrap();
    if let Some(position) = args.iter().position(|arg| arg == "--input") {
        let content = std::fs::read_to_string(&args[position + 1]).unwrap_or_default();
        let mut inputs = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("gh.inputs"))
            .unwrap();
        writeln!(inputs, "{}", content.replace('\n', " ")).unwrap();
    }
    let rules = std::fs::read_to_string(dir.join("gh.rules")).unwrap_or_default();
    for line in rules.lines() {
        let mut parts = line.splitn(2, '\t');
        let needle = parts.next().unwrap_or("");
        let stdout_file = parts.next().unwrap_or("");
        if !needle.is_empty() && joined.contains(needle) {
            print!("{}", std::fs::read_to_string(stdout_file).unwrap_or_default());
            std::process::exit(0);
        }
    }
    eprintln!("unscripted gh call");
    std::process::exit(1);
}
"#;

struct FakeGh {
    dir: PathBuf,
}

impl FakeGh {
    fn new(ws: &TempWorkspace) -> Self {
        let dir = ws.path().join("bin");
        fs::create_dir_all(&dir).expect("create shim dir");
        gh_shim::install_compiled_shim(&dir, SHIM_SOURCE);
        Self { dir }
    }

    fn script(&self, replies: &[(&str, &str)]) {
        let mut rules = Vec::new();
        for (index, (needle, stdout)) in replies.iter().enumerate() {
            let stdout_path = self.dir.join(format!("out{index}.txt"));
            fs::write(&stdout_path, stdout).expect("write canned stdout");
            rules.push(format!("{needle}\t{}", stdout_path.display()));
        }
        fs::write(self.dir.join(RULES_FILE), rules.join("\n")).expect("write rules");
    }

    fn lines(&self, file: &str) -> Vec<String> {
        fs::read_to_string(self.dir.join(file))
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

fn review_post_args(findings: &str) -> Vec<&str> {
    vec![
        REVIEW_POST,
        REPO_SLUG,
        PR_NUMBER,
        FINDINGS_FLAG,
        findings,
        PASS_FLAG,
        PASS,
        RUN_FLAG,
        RUN_ID,
    ]
}

fn write_findings(ws: &TempWorkspace) -> String {
    let path = ws.path().join(FINDINGS_FILE);
    fs::write(&path, FINDINGS).expect("write findings");
    path.to_string_lossy().to_string()
}

fn first_post_scripts() -> Vec<(&'static str, &'static str)> {
    vec![
        (INPUT_NEEDLE, SUBMIT_REPLY),
        (REVIEWS_NEEDLE, NO_REVIEWS),
        (FILES_NEEDLE, PR_FILES),
        (PULL_NEEDLE, HEAD_REPLY),
        (PR_VIEW_NEEDLE, PR_VIEW_REPLY),
        (THREADS_NEEDLE, NO_THREADS),
    ]
}

fn replay_scripts() -> Vec<(&'static str, &'static str)> {
    vec![
        (INPUT_NEEDLE, SUBMIT_REPLY),
        (REVIEWS_NEEDLE, MARKED_REVIEWS),
        (FILES_NEEDLE, PR_FILES),
        (PULL_NEEDLE, HEAD_REPLY),
        (PR_VIEW_NEEDLE, PR_VIEW_REPLY),
        (THREADS_NEEDLE, MARKED_THREADS),
    ]
}

/// Two in-diff findings become inline comments, the third goes in the body.
#[test]
fn review_post_splits_inline_comments_from_the_body_entry() {
    let ws = workspace();
    let gh = FakeGh::new(&ws);
    gh.script(&first_post_scripts());
    let findings = write_findings(&ws);
    let output = gh.run(&ws, &review_post_args(&findings));
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let outcome = stdout_json(&output);
    assert_eq!(outcome["posted"], true);
    assert_eq!(outcome["reviewId"], 99);
    let inputs = gh.lines(INPUTS_FILE);
    assert_eq!(inputs.len(), 1);
    let review: Value = serde_json::from_str(&inputs[0]).expect("review payload");
    assert_eq!(review["comments"].as_array().expect("comments").len(), 2);
    let body = review["body"].as_str().expect("body");
    assert!(body.contains("id=F-3 run=run-7 pass=1"), "body: {body}");
}

/// A second identical call posts nothing and returns the existing threads.
#[test]
fn review_post_replay_makes_no_new_post() {
    let ws = workspace();
    let gh = FakeGh::new(&ws);
    gh.script(&replay_scripts());
    let findings = write_findings(&ws);
    let output = gh.run(&ws, &review_post_args(&findings));
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let outcome = stdout_json(&output);
    assert_eq!(outcome["posted"], false);
    assert_eq!(outcome["threads"].as_array().expect("threads").len(), 2);
    assert!(gh.lines(INPUTS_FILE).is_empty());
    assert!(!gh
        .lines(CALLS_FILE)
        .iter()
        .any(|call| call.contains(INPUT_NEEDLE)));
}

/// Thread reopen replies first, then unresolves the thread.
#[test]
fn thread_reopen_replies_then_unresolves() {
    let ws = workspace();
    let gh = FakeGh::new(&ws);
    gh.script(&[
        (REPLY_NEEDLE, EMPTY_OBJECT),
        (UNRESOLVE_NEEDLE, EMPTY_OBJECT),
    ]);
    let body_file = ws.path().join(REPLY_FILE);
    fs::write(&body_file, REPLY_BODY).expect("write reply");
    let body_path = body_file.to_string_lossy().to_string();
    let output = gh.run(&ws, &[THREAD_REOPEN, THREAD_ID, BODY_FILE_FLAG, &body_path]);
    assert_eq!(output.status.code(), Some(0), "{output:?}");
    let outcome = stdout_json(&output);
    assert_eq!(outcome["threadId"], THREAD_ID);
    assert_eq!(outcome["replied"], true);
    assert_eq!(outcome["unresolved"], true);
    let calls = gh.lines(CALLS_FILE);
    assert_eq!(calls.len(), 2);
    assert!(calls[0].contains(REPLY_NEEDLE));
    assert!(calls[1].contains(UNRESOLVE_NEEDLE));
}
