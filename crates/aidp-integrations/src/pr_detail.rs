//! Read-only GitHub access for a PR page: detail, paginated files, file diff and hunk checks.

use crate::gh::derive_review_thread;
use crate::models::comment_point::CommentPoint;
use crate::models::constants::process::GH_PROGRAM;
use crate::models::enums::diff_side::DiffSide;
use crate::models::pr_commit::PrCommit;
use crate::models::pr_detail::PrDetail;
use crate::models::pr_file::PrFile;
use crate::models::pr_files_page::PrFilesPage;
use crate::models::pr_review_summary::PrReviewSummary;
use crate::models::rest_comment_page::RestCommentPage;
use crate::models::rest_comment_user::RestCommentUser;
use crate::models::review_thread::ReviewThread;
use crate::points::split_points;
use crate::pr::{pr_status, PrStatus, ReviewState};
use crate::pr_comments::{is_bot, parse_comments};
use crate::IntegrationError;
use aidp_core::models::process::ProcessSpec;
use aidp_core::process_runner::ProcessRunner;
use serde_json::Value;
use std::collections::HashSet;

const DETAIL_QUERY: &str = r#"
query($owner:String!,$name:String!,$number:Int!,$cursor:String){
  viewer{ login }
  repository(owner:$owner,name:$name){
    pullRequest(number:$number){
      body
      commits(first:100){ nodes{ commit{ oid messageHeadline authoredDate author{ name user{ login } } } } }
      reviews(first:100){ nodes{ databaseId author{ login } state body submittedAt url commit{ oid } } }
      reviewRequests(first:50){ nodes{ requestedReviewer{ ... on User{ login } ... on Team{ name } } } }
      comments(first:100,after:$cursor){ pageInfo{ hasNextPage endCursor } nodes{ databaseId author{ login } body createdAt url } }
      reviewThreads(first:100){ nodes{
        id isResolved isOutdated path line startLine
        comments(first:50){ totalCount nodes{ id databaseId author{ login } body createdAt url } }
      } }
    }
  }
}"#;

const GH_API: &str = "api";
const GRAPHQL: &str = "graphql";
const FIELD_FLAG: &str = "-f";
const TYPED_FIELD_FLAG: &str = "-F";
const HEADER_FLAG: &str = "-H";
const DIFF_ACCEPT: &str = "Accept: application/vnd.github.v3.diff";
const DIFF_HEADER_PREFIX: &str = "diff --git ";
const HUNK_PREFIX: &str = "@@ -";
const HUNK_END: &str = " @@";
const HUNK_RIGHT_SEPARATOR: &str = " +";
const ADDED: char = '+';
const REMOVED: char = '-';
const NO_NEWLINE_MARKER: char = '\\';
const MAX_PATCH_BYTES: usize = 1024 * 1024;
const MAX_COMMENT_PAGES: usize = 50;
const MAX_FILES_PER_PAGE: u32 = 100;
const FIRST_PAGE: u32 = 1;
const EMPTY_PAGES: &str = "[]";
const BAD_SLUG: &str = "repository slug (expected owner/name)";
const NO_PR: &str = "pull request";
const FILE_NOT_IN_DIFF: &str = "file in the pull request diff";

const VIEWER_POINTER: &str = "/viewer/login";
const PR_POINTER: &str = "/repository/pullRequest";
const COMMENT_NODES_POINTER: &str = "/repository/pullRequest/comments/nodes";
const THREAD_NODES_POINTER: &str = "/repository/pullRequest/reviewThreads/nodes";
const HAS_NEXT_POINTER: &str = "/repository/pullRequest/comments/pageInfo/hasNextPage";
const END_CURSOR_POINTER: &str = "/repository/pullRequest/comments/pageInfo/endCursor";

/// Field names read from or written to GitHub payloads.
mod wire {
    pub const ADDITIONS: &str = "additions";
    pub const AUTHOR: &str = "author";
    pub const AUTHORED_DATE: &str = "authoredDate";
    pub const BODY: &str = "body";
    pub const COMMIT: &str = "commit";
    pub const COMMITS: &str = "commits";
    pub const CREATED_AT: &str = "createdAt";
    pub const DATABASE_ID: &str = "databaseId";
    pub const DATA: &str = "data";
    pub const DELETIONS: &str = "deletions";
    pub const ERRORS: &str = "errors";
    pub const FILENAME: &str = "filename";
    pub const LOGIN: &str = "login";
    pub const MESSAGE_HEADLINE: &str = "messageHeadline";
    pub const NAME: &str = "name";
    pub const NODES: &str = "nodes";
    pub const OID: &str = "oid";
    pub const PATCH: &str = "patch";
    pub const PREVIOUS_FILENAME: &str = "previous_filename";
    pub const REQUESTED_REVIEWER: &str = "requestedReviewer";
    pub const REVIEW_REQUESTS: &str = "reviewRequests";
    pub const REVIEWS: &str = "reviews";
    pub const SHA: &str = "sha";
    pub const STATE: &str = "state";
    pub const STATUS: &str = "status";
    pub const SUBMITTED_AT: &str = "submittedAt";
    pub const URL: &str = "url";
    pub const USER: &str = "user";
}

fn upstream(error: impl ToString) -> IntegrationError {
    IntegrationError::Upstream(error.to_string())
}

fn run_gh(runner: &dyn ProcessRunner, args: Vec<String>) -> Result<String, IntegrationError> {
    let spec = ProcessSpec::new(GH_PROGRAM).args(args);
    let output = runner.output(&spec)?;
    if !output.status.success() {
        return Err(upstream(String::from_utf8_lossy(&output.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn text(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

fn login_of(value: &Value) -> String {
    value
        .get(wire::AUTHOR)
        .and_then(|author| text(author, wire::LOGIN))
        .unwrap_or_default()
}

fn nodes_of<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value
        .get(key)
        .and_then(|connection| connection.get(wire::NODES))
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

fn split_slug(repo: &str) -> Result<(&str, &str), IntegrationError> {
    repo.split_once('/')
        .ok_or_else(|| IntegrationError::NotFound(BAD_SLUG.to_string()))
}

fn graphql_page(
    runner: &dyn ProcessRunner,
    repo: &str,
    number: u64,
    cursor: Option<&str>,
) -> Result<Value, IntegrationError> {
    let (owner, name) = split_slug(repo)?;
    let mut args = vec![
        GH_API.to_string(),
        GRAPHQL.to_string(),
        FIELD_FLAG.to_string(),
        format!("query={DETAIL_QUERY}"),
        FIELD_FLAG.to_string(),
        format!("owner={owner}"),
        FIELD_FLAG.to_string(),
        format!("name={name}"),
        TYPED_FIELD_FLAG.to_string(),
        format!("number={number}"),
    ];
    if let Some(after) = cursor {
        args.push(FIELD_FLAG.to_string());
        args.push(format!("cursor={after}"));
    }
    let body: Value = serde_json::from_str(&run_gh(runner, args)?).map_err(upstream)?;
    match body.get(wire::DATA) {
        Some(data) if !data.is_null() => Ok(data.clone()),
        _ => Err(upstream(
            body.get(wire::ERRORS)
                .map_or_else(|| NO_PR.to_string(), Value::to_string),
        )),
    }
}

fn next_cursor(page: &Value) -> Option<String> {
    if !page.pointer(HAS_NEXT_POINTER).and_then(Value::as_bool)? {
        return None;
    }
    page.pointer(END_CURSOR_POINTER)
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn comment_nodes(page: &Value) -> Vec<Value> {
    page.pointer(COMMENT_NODES_POINTER)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// Fetches the PR data in one GraphQL query, looping its cursor until every comment is read.
fn fetch_detail_data(
    runner: &dyn ProcessRunner,
    repo: &str,
    number: u64,
) -> Result<Value, IntegrationError> {
    let mut data = graphql_page(runner, repo, number, None)?;
    if data.pointer(PR_POINTER).is_none_or(Value::is_null) {
        return Err(IntegrationError::NotFound(format!("{NO_PR} #{number}")));
    }
    let mut comments = comment_nodes(&data);
    let mut cursor = next_cursor(&data);
    let mut pages = 1;
    while let Some(after) = cursor {
        if pages >= MAX_COMMENT_PAGES {
            break;
        }
        let page = graphql_page(runner, repo, number, Some(&after))?;
        comments.extend(comment_nodes(&page));
        cursor = next_cursor(&page);
        pages += 1;
    }
    if let Some(slot) = data.pointer_mut(COMMENT_NODES_POINTER) {
        *slot = Value::Array(comments);
    }
    Ok(data)
}

fn commits_of(pr: &Value) -> Vec<PrCommit> {
    nodes_of(pr, wire::COMMITS)
        .iter()
        .filter_map(|node| node.get(wire::COMMIT))
        .map(|commit| {
            let author = commit.get(wire::AUTHOR);
            PrCommit {
                sha: text(commit, wire::OID).unwrap_or_default(),
                message_headline: text(commit, wire::MESSAGE_HEADLINE).unwrap_or_default(),
                author: author
                    .and_then(|value| value.get(wire::USER))
                    .and_then(|user| text(user, wire::LOGIN))
                    .or_else(|| author.and_then(|value| text(value, wire::NAME))),
                authored_date: text(commit, wire::AUTHORED_DATE),
            }
        })
        .collect()
}

fn review_points(id: Option<u64>, body: &str) -> Vec<CommentPoint> {
    if body.trim().is_empty() {
        return Vec::new();
    }
    split_points(body)
        .into_iter()
        .map(|point| CommentPoint {
            id: id.map(|review_id| format!("{review_id}#{}", point.index)),
            ..point
        })
        .collect()
}

fn reviews_of(pr: &Value) -> Vec<PrReviewSummary> {
    nodes_of(pr, wire::REVIEWS)
        .iter()
        .map(|node| {
            let author = login_of(node);
            let id = node.get(wire::DATABASE_ID).and_then(Value::as_u64);
            PrReviewSummary {
                id,
                bot: is_bot(&author, &[]),
                author,
                state: node
                    .get(wire::STATE)
                    .and_then(|state| serde_json::from_value(state.clone()).ok())
                    .unwrap_or(ReviewState::Unknown),
                commit_sha: node
                    .get(wire::COMMIT)
                    .and_then(|commit| text(commit, wire::OID)),
                submitted_at: text(node, wire::SUBMITTED_AT),
                url: text(node, wire::URL),
                points: review_points(id, &text(node, wire::BODY).unwrap_or_default()),
            }
        })
        .collect()
}

/// Rebuilds the REST page shape so the existing comment parser can be reused.
fn rest_pages(nodes: &[Value]) -> String {
    let comments: Vec<RestCommentPage> = nodes
        .iter()
        .map(|node| RestCommentPage {
            id: node
                .get(wire::DATABASE_ID)
                .and_then(Value::as_u64)
                .unwrap_or_default(),
            user: RestCommentUser {
                login: login_of(node),
            },
            body: text(node, wire::BODY).unwrap_or_default(),
            html_url: text(node, wire::URL).unwrap_or_default(),
            created_at: text(node, wire::CREATED_AT).unwrap_or_default(),
        })
        .collect();
    serde_json::to_string(&[comments]).unwrap_or_else(|_| EMPTY_PAGES.to_string())
}

fn requested_of(pr: &Value) -> Vec<String> {
    nodes_of(pr, wire::REVIEW_REQUESTS)
        .iter()
        .filter_map(|node| node.get(wire::REQUESTED_REVIEWER))
        .filter_map(|reviewer| text(reviewer, wire::LOGIN).or_else(|| text(reviewer, wire::NAME)))
        .collect()
}

fn threads_of(data: &Value) -> Vec<ReviewThread> {
    let viewer = data
        .pointer(VIEWER_POINTER)
        .and_then(Value::as_str)
        .unwrap_or_default();
    let handed = HashSet::new();
    data.pointer(THREAD_NODES_POINTER)
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice)
        .iter()
        .map(|node| derive_review_thread(node, viewer, &handed))
        .collect()
}

/// Folds the paginated GraphQL data and an already-read status into the page document.
fn assemble(status: PrStatus, data: &Value) -> Result<PrDetail, IntegrationError> {
    let pr = data.pointer(PR_POINTER).unwrap_or(&Value::Null);
    let comments = parse_comments(&rest_pages(&comment_nodes(data)), EMPTY_PAGES, &[])?;
    Ok(PrDetail {
        status,
        description: text(pr, wire::BODY).unwrap_or_default(),
        commits: commits_of(pr),
        reviews: reviews_of(pr),
        comments,
        threads: threads_of(data),
        requested_reviewers: requested_of(pr),
    })
}

/// Reads the PR page with an already-read status; one paginated GraphQL query.
pub fn pr_detail_with_status(
    runner: &dyn ProcessRunner,
    repo: &str,
    number: u64,
    status: PrStatus,
) -> Result<PrDetail, IntegrationError> {
    assemble(status, &fetch_detail_data(runner, repo, number)?)
}

/// Reads the PR page: status, description, commits, reviews, comments, threads and reviewers.
pub fn pr_detail(
    runner: &dyn ProcessRunner,
    repo: &str,
    number: u64,
) -> Result<PrDetail, IntegrationError> {
    let status = pr_status(Some(repo), Some(number), None)?;
    pr_detail_with_status(runner, repo, number, status)
}

fn file_of(raw: &Value) -> PrFile {
    let patch = text(raw, wire::PATCH).filter(|patch| patch.len() <= MAX_PATCH_BYTES);
    let count = |key: &str| raw.get(key).and_then(Value::as_u64).unwrap_or_default();
    PrFile {
        filename: text(raw, wire::FILENAME).unwrap_or_default(),
        status: text(raw, wire::STATUS).unwrap_or_default(),
        additions: count(wire::ADDITIONS),
        deletions: count(wire::DELETIONS),
        too_large: patch.is_none(),
        patch,
        previous_filename: text(raw, wire::PREVIOUS_FILENAME),
        sha: text(raw, wire::SHA),
    }
}

/// Parses one REST page of PR files; a full page means another may follow.
pub fn parse_files_page(
    body: &str,
    page: u32,
    per_page: u32,
) -> Result<PrFilesPage, IntegrationError> {
    let raw: Vec<Value> = serde_json::from_str(body).map_err(upstream)?;
    Ok(PrFilesPage {
        page,
        per_page,
        has_more: raw.len() >= per_page as usize,
        files: raw.iter().map(file_of).collect(),
    })
}

/// One page of the PR's files; a missing or over 1 MiB patch is dropped and flagged `too_large`.
pub fn pr_files(
    runner: &dyn ProcessRunner,
    repo: &str,
    number: u64,
    page: u32,
    per_page: u32,
) -> Result<PrFilesPage, IntegrationError> {
    let page = page.max(FIRST_PAGE);
    let per_page = per_page.clamp(FIRST_PAGE, MAX_FILES_PER_PAGE);
    let path = format!("repos/{repo}/pulls/{number}/files?per_page={per_page}&page={page}");
    let body = run_gh(runner, vec![GH_API.to_string(), path])?;
    parse_files_page(&body, page, per_page)
}

/// Cuts the section of `path` out of a full unified PR diff.
pub fn slice_file_diff(full_diff: &str, path: &str) -> Option<String> {
    let suffix = format!(" b/{path}");
    let mut section = String::new();
    let mut inside = false;
    for line in full_diff.split_inclusive('\n') {
        if line.starts_with(DIFF_HEADER_PREFIX) {
            if inside {
                break;
            }
            inside = line.trim_end().ends_with(&suffix);
        }
        if inside {
            section.push_str(line);
        }
    }
    (!section.is_empty()).then_some(section)
}

/// The full diff of one file of the PR, however large.
pub fn pr_file_diff(
    runner: &dyn ProcessRunner,
    repo: &str,
    number: u64,
    path: &str,
) -> Result<String, IntegrationError> {
    let args = vec![
        GH_API.to_string(),
        HEADER_FLAG.to_string(),
        DIFF_ACCEPT.to_string(),
        format!("repos/{repo}/pulls/{number}"),
    ];
    slice_file_diff(&run_gh(runner, args)?, path)
        .ok_or_else(|| IntegrationError::NotFound(FILE_NOT_IN_DIFF.to_string()))
}

fn hunk_starts(line: &str) -> Option<(u64, u64)> {
    let header = line.strip_prefix(HUNK_PREFIX)?;
    let ranges = header.split(HUNK_END).next()?;
    let (left, right) = ranges.split_once(HUNK_RIGHT_SEPARATOR)?;
    let start = |range: &str| range.split(',').next()?.parse().ok();
    Some((start(left)?, start(right)?))
}

/// For each hunk, the line numbers that exist on `side`.
fn side_lines_per_hunk(patch: &str, side: DiffSide) -> Vec<Vec<u64>> {
    let mut hunks: Vec<Vec<u64>> = Vec::new();
    let (mut left, mut right) = (0, 0);
    for line in patch.lines() {
        if let Some((left_start, right_start)) = hunk_starts(line) {
            (left, right) = (left_start, right_start);
            hunks.push(Vec::new());
            continue;
        }
        let Some(current) = hunks.last_mut() else {
            continue;
        };
        match line.chars().next() {
            Some(ADDED) => {
                if side == DiffSide::Right {
                    current.push(right);
                }
                right += 1;
            }
            Some(REMOVED) => {
                if side == DiffSide::Left {
                    current.push(left);
                }
                left += 1;
            }
            Some(NO_NEWLINE_MARKER) => {}
            _ => {
                current.push(if side == DiffSide::Left { left } else { right });
                left += 1;
                right += 1;
            }
        }
    }
    hunks
}

/// True when `line` (and `start_line` of a range) lies in one hunk of `patch` on `side`.
pub fn line_in_diff(patch: &str, line: u64, side: DiffSide, start_line: Option<u64>) -> bool {
    let start = start_line.unwrap_or(line);
    start <= line
        && side_lines_per_hunk(patch, side)
            .iter()
            .any(|hunk| hunk.contains(&start) && hunk.contains(&line))
}

#[cfg(test)]
#[path = "tests/units/pr_detail_tests.rs"]
mod pr_detail_tests;
