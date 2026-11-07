//! End-to-end tests of `aidp skill-rules check` against throwaway git repositories.

use crate::common::*;
use std::fs;
use std::path::Path;
use std::process::Command as StdCommand;

const GIT_PROGRAM: &str = "git";
const BASE_BRANCH: &str = "master";
const SOURCE_DIR: &str = "src";
const SOURCE_FILE: &str = "src/lib.rs";
const REPO_DIR: &str = "repo";
const FINDINGS_FILE: &str = "f.json";
const MERGED_FILE: &str = "merged.json";
const SKILL_RULES_ARG: &str = "skill-rules";
const CHECK_ARG: &str = "check";
const REPO_FLAG: &str = "--repo";
const BASE_FLAG: &str = "--base";
const FINDINGS_ARG: &str = "findings";
const MERGE_ARG: &str = "merge";
const LABEL_FLAG: &str = "--label";
const LABEL_AXIS: &str = "skill_rules";
const BASE_SOURCE: &str = "pub fn one() {}\n";
const LINE_COMMENT: &str = "    // explain";
const LINE_DOC_OUTER: &str = "/// doc";
const LINE_DOC_INNER: &str = "//! doc";
const TICKET_PREFIX: &str = "LOC";
const TICKET_NUMBER: &str = "123";
const ADDED_LINE_NUMBER: i64 = 2;
const KEY_FINDINGS: &str = "findings";
const KEY_FILE: &str = "file";
const KEY_LINE: &str = "line";
const KEY_SEVERITY: &str = "severity";
const KEY_BLOCKING: &str = "has_blocking_findings";
const SEVERITY_BLOCKING: &str = "blocking";
const CONFIG_FLAG: &str = "-c";
const GIT_IDENTITY_ARGS: [&str; 4] = [
    CONFIG_FLAG,
    "user.name=test",
    CONFIG_FLAG,
    "user.email=test@example.com",
];
const GIT_INIT: &str = "init";
const GIT_BRANCH_FLAG: &str = "-b";
const GIT_ADD: &str = "add";
const GIT_ADD_ALL_PATH: &str = ".";
const GIT_COMMIT: &str = "commit";
const GIT_MESSAGE_FLAG: &str = "-m";
const BASE_COMMIT_MESSAGE: &str = "base";

fn git(repo: &Path, args: &[&str]) {
    let output = StdCommand::new(GIT_PROGRAM)
        .args(GIT_IDENTITY_ARGS)
        .args(args)
        .current_dir(repo)
        .output()
        .expect("git should run");
    assert!(
        output.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
}

fn init_repo(ws: &aidp_testkit::TempWorkspace) -> std::path::PathBuf {
    let repo = ws.child(REPO_DIR);
    fs::create_dir_all(repo.join(SOURCE_DIR)).expect("create src dir");
    git(&repo, &[GIT_INIT, GIT_BRANCH_FLAG, BASE_BRANCH]);
    fs::write(repo.join(SOURCE_FILE), BASE_SOURCE).expect("write base source");
    git(&repo, &[GIT_ADD, GIT_ADD_ALL_PATH]);
    git(&repo, &[GIT_COMMIT, GIT_MESSAGE_FLAG, BASE_COMMIT_MESSAGE]);
    repo
}

fn add_lines(repo: &Path, lines: &[&str]) {
    let mut content = String::from(BASE_SOURCE);
    for line in lines {
        content.push_str(line);
        content.push('\n');
    }
    fs::write(repo.join(SOURCE_FILE), content).expect("write changed source");
}

fn run_check(ws: &aidp_testkit::TempWorkspace, repo: &Path) -> serde_json::Value {
    let findings_path = ws.child(FINDINGS_FILE);
    let mut cmd = aidp(ws);
    cmd.arg(SKILL_RULES_ARG)
        .arg(CHECK_ARG)
        .arg(REPO_FLAG)
        .arg(repo)
        .arg(BASE_FLAG)
        .arg(BASE_BRANCH)
        .arg(OUT_FLAG)
        .arg(&findings_path);
    let output = cmd.output().expect("skill-rules check should run");
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = fs::read_to_string(&findings_path).expect("findings file written");
    serde_json::from_str(&text).expect("findings file is JSON")
}

fn finding_list(report: &serde_json::Value) -> &Vec<serde_json::Value> {
    report[KEY_FINDINGS]
        .as_array()
        .expect("findings is an array")
}

#[test]
fn line_comment_yields_one_blocking_finding_with_exact_line() {
    let ws = workspace();
    let repo = init_repo(&ws);
    add_lines(&repo, &[LINE_COMMENT]);

    let report = run_check(&ws, &repo);

    assert_eq!(report[KEY_BLOCKING], true);
    let list = finding_list(&report);
    assert_eq!(list.len(), 1);
    assert_eq!(list[0][KEY_FILE], SOURCE_FILE);
    assert_eq!(list[0][KEY_LINE], ADDED_LINE_NUMBER);
    assert_eq!(list[0][KEY_SEVERITY], SEVERITY_BLOCKING);
}

#[test]
fn doc_comments_yield_no_finding() {
    let ws = workspace();
    let repo = init_repo(&ws);
    add_lines(&repo, &[LINE_DOC_OUTER, LINE_DOC_INNER]);

    let report = run_check(&ws, &repo);

    assert_eq!(report[KEY_BLOCKING], false);
    assert_eq!(finding_list(&report).len(), 0);
}

#[test]
fn ticket_key_yields_one_blocking_finding() {
    let ws = workspace();
    let repo = init_repo(&ws);
    let ticket_line = format!("let key = \"{TICKET_PREFIX}-{TICKET_NUMBER}\";");
    add_lines(&repo, &[ticket_line.as_str()]);

    let report = run_check(&ws, &repo);

    assert_eq!(report[KEY_BLOCKING], true);
    let list = finding_list(&report);
    assert_eq!(list.len(), 1);
    assert_eq!(list[0][KEY_FILE], SOURCE_FILE);
    assert_eq!(list[0][KEY_LINE], ADDED_LINE_NUMBER);
}

#[test]
fn skill_rules_output_passes_findings_merge() {
    let ws = workspace();
    let repo = init_repo(&ws);
    add_lines(&repo, &[LINE_COMMENT]);
    run_check(&ws, &repo);

    let mut cmd = aidp(&ws);
    cmd.arg(FINDINGS_ARG)
        .arg(MERGE_ARG)
        .arg(LABEL_FLAG)
        .arg(format!("{LABEL_AXIS}={FINDINGS_FILE}"))
        .arg(OUT_FLAG)
        .arg(MERGED_FILE);
    let output = cmd.output().expect("findings merge should run");
    assert_eq!(
        output.status.code(),
        Some(0),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let merged_text = fs::read_to_string(ws.child(MERGED_FILE)).expect("merged file written");
    let merged: serde_json::Value = serde_json::from_str(&merged_text).expect("merged is JSON");
    assert_eq!(merged[KEY_BLOCKING], true);
}
