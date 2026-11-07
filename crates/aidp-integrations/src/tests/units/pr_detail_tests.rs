use super::*;
use crate::pr::{build_status, BranchRules};
use aidp_core::models::process::ScriptedOutput;
use aidp_core::process_runner::{ScriptedProcessRunner, SpawnedChild};
use serde_json::json;
use std::collections::VecDeque;
use std::process::Output;
use std::sync::Mutex;

const DETAIL: &str = include_str!("fixtures/pr_detail/detail.json");
const FILES: &str = include_str!("fixtures/pr_detail/files.json");
const STATUS_DATA: &str = include_str!("fixtures/pr/graphql_open_pending.json");

const REPO: &str = "o/r";
const READ_AT: &str = "2026-09-28T14:00:00Z";
const PATCH: &str =
    "@@ -10,4 +10,5 @@\n ctx10\n-del11\n+add11\n+add12\n ctx12\n@@ -40,2 +41,2 @@\n c40\n c41";
const COMMENTS_PER_PAGE: usize = 100;

struct QueuedRunner {
    replies: Mutex<VecDeque<String>>,
    calls: Mutex<Vec<Vec<String>>>,
}

impl QueuedRunner {
    fn new(replies: Vec<String>) -> Self {
        Self {
            replies: Mutex::new(replies.into()),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn call_count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }

    fn args_of(&self, call: usize) -> Vec<String> {
        self.calls.lock().unwrap()[call].clone()
    }
}

impl ProcessRunner for QueuedRunner {
    fn output(&self, spec: &ProcessSpec) -> std::io::Result<Output> {
        self.calls.lock().unwrap().push(spec.args.clone());
        let reply = self.replies.lock().unwrap().pop_front().unwrap_or_default();
        ScriptedProcessRunner::new()
            .with_output(ScriptedOutput {
                status_code: 0,
                stdout: reply.into_bytes(),
                stderr: Vec::new(),
            })
            .output(spec)
    }

    fn spawn(&self, _spec: &ProcessSpec) -> std::io::Result<Box<dyn SpawnedChild>> {
        Err(std::io::Error::other("spawn is not scripted"))
    }
}

fn scripted(stdout: &str) -> ScriptedProcessRunner {
    ScriptedProcessRunner::new().with_output(ScriptedOutput {
        status_code: 0,
        stdout: stdout.as_bytes().to_vec(),
        stderr: Vec::new(),
    })
}

fn status() -> PrStatus {
    let data: Value = serde_json::from_str(STATUS_DATA).unwrap();
    build_status(
        REPO,
        &data,
        None,
        BranchRules::default(),
        READ_AT.to_string(),
    )
    .unwrap()
}

fn comment_page(first_id: usize, has_next: bool, end_cursor: &str) -> String {
    let nodes: Vec<Value> = (first_id..first_id + COMMENTS_PER_PAGE)
        .map(|id| {
            json!({"databaseId": id, "author": {"login": "carol"}, "body": format!("note {id}"),
                   "createdAt": "2026-09-28T11:00:00Z", "url": format!("u{id}")})
        })
        .collect();
    json!({"data": {"viewer": {"login": "me"}, "repository": {"pullRequest": {
        "body": "d",
        "comments": {"pageInfo": {"hasNextPage": has_next, "endCursor": end_cursor}, "nodes": nodes}
    }}}})
    .to_string()
}

#[test]
fn detail_aggregates_every_part() {
    let detail = pr_detail_with_status(&scripted(DETAIL), REPO, 7, status()).unwrap();
    assert_eq!(detail.description, "Adds the thing");
    assert_eq!(detail.commits[0].sha, "abc123");
    assert_eq!(detail.commits[0].author.as_deref(), Some("dev"));
    assert_eq!(detail.requested_reviewers, vec!["bob", "core-team"]);
    let review = &detail.reviews[0];
    assert_eq!(review.state, ReviewState::ChangesRequested);
    assert_eq!(review.commit_sha.as_deref(), Some("abc123"));
    assert_eq!(review.points.len(), 2);
    assert_eq!(review.points[1].id.as_deref(), Some("900#1"));
    assert_eq!(detail.comments.len(), 2);
    assert_eq!(detail.comments[0].points.len(), 2);
    assert!(detail.comments[1].bot);
    assert_eq!(detail.threads.len(), 1);
    assert!(detail.threads[0].comments[1].is_mine);
    assert!(!detail.threads[0].comments[0].is_mine);
}

#[test]
fn two_hundred_comments_cost_one_paginated_query() {
    let runner = QueuedRunner::new(vec![
        comment_page(1, true, "cursor-1"),
        comment_page(101, false, ""),
    ]);
    let detail = pr_detail_with_status(&runner, REPO, 7, status()).unwrap();
    assert_eq!(detail.comments.len(), 200);
    assert_eq!(runner.call_count(), 2);
    assert!(!runner
        .args_of(0)
        .iter()
        .any(|arg| arg.starts_with("cursor=")));
    assert!(runner.args_of(1).contains(&"cursor=cursor-1".to_string()));
    assert_eq!(runner.args_of(0)[3], runner.args_of(1)[3]);
}

#[test]
fn missing_pull_request_is_not_found() {
    let reply = json!({"data": {"repository": {"pullRequest": null}}}).to_string();
    let error = pr_detail_with_status(&scripted(&reply), REPO, 7, status()).unwrap_err();
    assert!(matches!(error, IntegrationError::NotFound(_)));
}

#[test]
fn malformed_slug_is_rejected_before_any_call() {
    let runner = QueuedRunner::new(Vec::new());
    let error = pr_detail_with_status(&runner, "norepo", 7, status()).unwrap_err();
    assert!(matches!(error, IntegrationError::NotFound(_)));
    assert_eq!(runner.call_count(), 0);
}

#[test]
fn failed_gh_call_is_upstream() {
    let runner = ScriptedProcessRunner::new().with_output(ScriptedOutput {
        status_code: 1,
        stdout: Vec::new(),
        stderr: b"HTTP 502".to_vec(),
    });
    let error = pr_files(&runner, REPO, 7, 1, 30).unwrap_err();
    assert!(matches!(error, IntegrationError::Upstream(message) if message == "HTTP 502"));
}

#[test]
fn files_flag_missing_patch_as_too_large() {
    let page = pr_files(&scripted(FILES), REPO, 7, 1, 3).unwrap();
    assert!(page.has_more);
    assert_eq!(page.files.len(), 3);
    let modified = &page.files[0];
    assert!(!modified.too_large);
    assert_eq!(modified.sha.as_deref(), Some("blob1"));
    assert_eq!((modified.additions, modified.deletions), (2, 1));
    assert!(page.files[1].too_large);
    assert!(page.files[1].patch.is_none());
    assert_eq!(
        page.files[2].previous_filename.as_deref(),
        Some("src/old_name.rs")
    );
}

#[test]
fn files_flag_patch_over_one_mebibyte() {
    let huge = "x".repeat(MAX_PATCH_BYTES + 1);
    let exact = "y".repeat(MAX_PATCH_BYTES);
    let body = json!([
        {"filename": "big", "status": "added", "patch": huge},
        {"filename": "edge", "status": "added", "patch": exact}
    ])
    .to_string();
    let page = pr_files(&scripted(&body), REPO, 7, 1, 30).unwrap();
    assert!(page.files[0].too_large && page.files[0].patch.is_none());
    assert!(!page.files[1].too_large && page.files[1].patch.is_some());
    assert!(!page.has_more);
}

#[test]
fn files_request_clamps_paging() {
    let runner = QueuedRunner::new(vec!["[]".to_string()]);
    let page = pr_files(&runner, REPO, 7, 0, 500).unwrap();
    assert_eq!((page.page, page.per_page), (1, 100));
    assert_eq!(
        runner.args_of(0)[1],
        "repos/o/r/pulls/7/files?per_page=100&page=1"
    );
}

#[test]
fn file_diff_is_sliced_from_the_full_diff() {
    let full = "diff --git a/a.rs b/a.rs\n@@ -1 +1 @@\n-1\n+2\ndiff --git a/dir/b.rs b/dir/b.rs\n@@ -1 +1 @@\n-x\n+y\n";
    let diff = pr_file_diff(&scripted(full), REPO, 7, "dir/b.rs").unwrap();
    assert_eq!(
        diff,
        "diff --git a/dir/b.rs b/dir/b.rs\n@@ -1 +1 @@\n-x\n+y\n"
    );
    let missing = pr_file_diff(&scripted(full), REPO, 7, "b.rs").unwrap_err();
    assert!(matches!(missing, IntegrationError::NotFound(_)));
}

#[test]
fn right_side_lines_follow_added_and_context() {
    assert!(line_in_diff(PATCH, 10, DiffSide::Right, None));
    assert!(line_in_diff(PATCH, 12, DiffSide::Right, None));
    assert!(line_in_diff(PATCH, 13, DiffSide::Right, None));
    assert!(line_in_diff(PATCH, 42, DiffSide::Right, None));
    assert!(!line_in_diff(PATCH, 14, DiffSide::Right, None));
}

#[test]
fn left_side_lines_follow_removed_and_context() {
    assert!(line_in_diff(PATCH, 11, DiffSide::Left, None));
    assert!(line_in_diff(PATCH, 12, DiffSide::Left, None));
    assert!(!line_in_diff(PATCH, 13, DiffSide::Left, None));
    assert!(line_in_diff(PATCH, 41, DiffSide::Left, None));
}

#[test]
fn line_between_hunks_is_outside_the_diff() {
    assert!(!line_in_diff(PATCH, 25, DiffSide::Right, None));
    assert!(!line_in_diff(PATCH, 25, DiffSide::Left, None));
    assert!(!line_in_diff("", 1, DiffSide::Right, None));
}

#[test]
fn ranges_must_stay_inside_one_hunk() {
    assert!(line_in_diff(PATCH, 13, DiffSide::Right, Some(10)));
    assert!(!line_in_diff(PATCH, 42, DiffSide::Right, Some(12)));
    assert!(!line_in_diff(PATCH, 10, DiffSide::Right, Some(13)));
}

#[test]
fn hunk_header_without_counts_is_parsed() {
    let patch = "@@ -3 +5 @@\n-old\n+new";
    assert!(line_in_diff(patch, 3, DiffSide::Left, None));
    assert!(line_in_diff(patch, 5, DiffSide::Right, None));
    assert!(!line_in_diff(patch, 5, DiffSide::Left, None));
}

/// Serialized REST pages parse back through the comment parser with their fields intact.
#[test]
fn rest_pages_round_trip_through_the_comment_parser() {
    let nodes = vec![json!({
        "databaseId": 42,
        "author": {"login": "alice"},
        "body": "looks good",
        "url": "https://example/c/42",
        "createdAt": "2026-09-28T14:00:00Z"
    })];
    let comments = parse_comments(&rest_pages(&nodes), EMPTY_PAGES, &[]).unwrap();
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0].author, "alice");
}
