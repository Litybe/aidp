//! Posts review findings as one idempotent GitHub review, and reopens review threads.

use crate::diff_hunks::{anchor_in_hunks, parse_hunks};
use crate::gh::pr_review_threads;
use crate::models::constants::process::GH_PROGRAM;
use crate::models::constants::review_post::{
    BAD_REVIEWS_REPLY, BODY_KEY, COMMENT_HEADING_CLOSE, COMMENT_HEADING_OPEN, DEFAULT_SEVERITY,
    FILES_PER_PAGE, FIRST_PAGE, GH_API, HEAD_SHA_POINTER, INVALID_FINDINGS, MAX_FILE_PAGES,
    NO_HEAD_SHA, NO_LOCATION, REVIEW_OUT_OF_DIFF_HEADING, REVIEW_SUMMARY, THREADS_KEY,
    THREAD_ID_KEY, THREAD_LINE_KEY, THREAD_PATH_KEY,
};
use crate::models::diff_hunk::DiffHunk;
use crate::models::enums::review_event::ReviewEvent;
use crate::models::finding_location::FindingLocation;
use crate::models::finding_marker::FindingMarker;
use crate::models::posted_thread::PostedThread;
use crate::models::pr_file::PrFile;
use crate::models::review_draft::ReviewDraft;
use crate::models::review_finding::ReviewFinding;
use crate::models::review_findings_file::ReviewFindingsFile;
use crate::models::review_post_outcome::ReviewPostOutcome;
use crate::models::thread_reopen_outcome::ThreadReopenOutcome;
use crate::models::thread_result::ThreadResult;
use crate::pr_actions::{thread_reply, thread_resolve};
use crate::pr_detail::pr_files;
use crate::pr_writes::review_submit;
use crate::IntegrationError;
use aidp_core::models::process::ProcessSpec;
use aidp_core::process_runner::ProcessRunner;
use serde_json::Value;
use std::collections::HashSet;
use std::path::Path;

type ThreadReader<'a> = &'a dyn Fn() -> Result<Value, IntegrationError>;

fn upstream(error: impl ToString) -> IntegrationError {
    IntegrationError::Upstream(error.to_string())
}

fn run_gh(runner: &dyn ProcessRunner, path: String) -> Result<Value, IntegrationError> {
    let spec = ProcessSpec::new(GH_PROGRAM).args(vec![GH_API.to_string(), path]);
    let output = runner.output(&spec)?;
    if !output.status.success() {
        return Err(upstream(String::from_utf8_lossy(&output.stderr).trim()));
    }
    serde_json::from_slice(&output.stdout).map_err(upstream)
}

fn load_findings(path: &Path) -> Result<Vec<ReviewFinding>, IntegrationError> {
    let text = std::fs::read_to_string(path)?;
    let file: ReviewFindingsFile = serde_json::from_str(&text)
        .map_err(|_| IntegrationError::Conflict(INVALID_FINDINGS.to_string()))?;
    Ok(file.findings)
}

/// Threads of the PR whose first comment carries a marker, as read from the threads reply.
fn marked_threads(reply: &Value) -> Vec<(FindingMarker, PostedThread)> {
    let threads = reply
        .get(THREADS_KEY)
        .and_then(Value::as_array)
        .map_or(&[][..], Vec::as_slice);
    threads
        .iter()
        .filter_map(|thread| {
            let body = thread.get(BODY_KEY)?.as_str()?;
            let marker = FindingMarker::parse(body)?;
            let thread_id = thread.get(THREAD_ID_KEY)?.as_str()?.to_string();
            let posted = PostedThread {
                finding_id: marker.id.clone(),
                thread_id,
                path: thread
                    .get(THREAD_PATH_KEY)
                    .and_then(Value::as_str)
                    .map(str::to_string),
                line: thread.get(THREAD_LINE_KEY).and_then(Value::as_i64),
            };
            Some((marker, posted))
        })
        .collect()
}

fn same_pass(marker: &FindingMarker, run_id: &str, pass: u32) -> bool {
    marker.run == run_id && marker.pass == pass
}

fn pass_threads(reply: &Value, run_id: &str, pass: u32) -> Vec<PostedThread> {
    marked_threads(reply)
        .into_iter()
        .filter(|(marker, _)| same_pass(marker, run_id, pass))
        .map(|(_, thread)| thread)
        .collect()
}

fn review_body_markers(
    runner: &dyn ProcessRunner,
    repo: &str,
    number: u64,
) -> Result<Vec<FindingMarker>, IntegrationError> {
    let reviews = run_gh(runner, format!("repos/{repo}/pulls/{number}/reviews"))?;
    let list = reviews
        .as_array()
        .ok_or_else(|| upstream(BAD_REVIEWS_REPLY))?;
    Ok(list
        .iter()
        .filter_map(|review| review.get(BODY_KEY).and_then(Value::as_str))
        .flat_map(FindingMarker::parse_all)
        .collect())
}

fn all_files(
    runner: &dyn ProcessRunner,
    repo: &str,
    number: u64,
) -> Result<Vec<PrFile>, IntegrationError> {
    let mut files = Vec::new();
    for page in FIRST_PAGE..FIRST_PAGE + MAX_FILE_PAGES {
        let batch = pr_files(runner, repo, number, page, FILES_PER_PAGE)?;
        files.extend(batch.files);
        if !batch.has_more {
            break;
        }
    }
    Ok(files)
}

fn head_sha(
    runner: &dyn ProcessRunner,
    repo: &str,
    number: u64,
) -> Result<String, IntegrationError> {
    let pull = run_gh(runner, format!("repos/{repo}/pulls/{number}"))?;
    pull.pointer(HEAD_SHA_POINTER)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| IntegrationError::NotFound(NO_HEAD_SHA.to_string()))
}

/// Hunks of the file at `path`; a path that was renamed in the PR resolves to its new name.
fn hunks_of(files: &[PrFile], path: &str) -> Option<(String, Vec<DiffHunk>)> {
    let file = files
        .iter()
        .find(|file| file.filename == path)
        .or_else(|| {
            files
                .iter()
                .find(|file| file.previous_filename.as_deref() == Some(path))
        })?;
    let patch = file.patch.as_deref()?;
    Some((file.filename.clone(), parse_hunks(patch)))
}

/// The location moved onto the PR's file name when it lies inside the diff.
fn anchor_in_diff(files: &[PrFile], anchor: &FindingLocation) -> Option<FindingLocation> {
    let (path, hunks) = hunks_of(files, &anchor.path)?;
    anchor_in_hunks(&hunks, anchor.line, anchor.side, anchor.start_line).then(|| FindingLocation {
        path,
        ..anchor.clone()
    })
}

fn heading(finding: &ReviewFinding) -> String {
    let severity = finding.severity.as_deref().unwrap_or(DEFAULT_SEVERITY);
    format!(
        "{COMMENT_HEADING_OPEN}{severity}{COMMENT_HEADING_CLOSE} {}",
        finding.title
    )
}

fn comment_body(finding: &ReviewFinding, marker: &FindingMarker) -> String {
    let mut body = format!(
        "{}
{}",
        marker.render(),
        heading(finding)
    );
    if let Some(description) = &finding.description {
        body.push_str(
            "

",
        );
        body.push_str(description);
    }
    body
}

fn draft_of(
    finding: &ReviewFinding,
    marker: &FindingMarker,
    anchor: FindingLocation,
) -> ReviewDraft {
    let start_line = anchor.start_line.filter(|start| *start != anchor.line);
    ReviewDraft {
        id: finding.id.clone(),
        path: anchor.path,
        line: anchor.line,
        side: anchor.side,
        start_line,
        start_side: start_line.map(|_| anchor.side),
        body: comment_body(finding, marker),
    }
}

fn out_of_diff_entry(finding: &ReviewFinding, marker: &FindingMarker) -> String {
    let location = finding.anchor().map_or_else(
        || NO_LOCATION.to_string(),
        |anchor| format!("`{}:{}`", anchor.path, anchor.line),
    );
    format!("- {} {location} {}", marker.render(), heading(finding))
}

fn review_body(pass: u32, inline_count: usize, demoted: &[String]) -> String {
    let mut body = format!("{REVIEW_SUMMARY} pass {pass}: {inline_count} inline finding(s)");
    if !demoted.is_empty() {
        body.push_str(&format!("\n\n{REVIEW_OUT_OF_DIFF_HEADING}\n"));
        body.push_str(&demoted.join("\n"));
    }
    body
}

/// Posts the findings of one pass as a single COMMENT review, using the supplied thread reader.
pub fn review_post_with(
    runner: &dyn ProcessRunner,
    read_threads: ThreadReader<'_>,
    repo: &str,
    number: u64,
    findings_path: &Path,
    pass: u32,
    run_id: &str,
) -> Result<ReviewPostOutcome, IntegrationError> {
    let findings = load_findings(findings_path)?;
    let before = read_threads()?;
    let mut posted_ids: HashSet<String> = marked_threads(&before)
        .into_iter()
        .filter(|(marker, _)| same_pass(marker, run_id, pass))
        .map(|(marker, _)| marker.id)
        .collect();
    posted_ids.extend(
        review_body_markers(runner, repo, number)?
            .into_iter()
            .filter(|marker| same_pass(marker, run_id, pass))
            .map(|marker| marker.id),
    );
    let pending: Vec<&ReviewFinding> = findings
        .iter()
        .filter(|finding| !posted_ids.contains(&finding.id))
        .collect();
    if pending.is_empty() {
        return Ok(ReviewPostOutcome {
            posted: false,
            review_id: None,
            threads: pass_threads(&before, run_id, pass),
        });
    }
    let sha = head_sha(runner, repo, number)?;
    let files = all_files(runner, repo, number)?;
    let mut drafts = Vec::new();
    let mut demoted = Vec::new();
    for finding in pending {
        let marker = FindingMarker {
            id: finding.id.clone(),
            run: run_id.to_string(),
            pass,
        };
        match finding
            .anchor()
            .and_then(|anchor| anchor_in_diff(&files, &anchor))
        {
            Some(anchor) => drafts.push(draft_of(finding, &marker, anchor)),
            None => demoted.push(out_of_diff_entry(finding, &marker)),
        }
    }
    let body = review_body(pass, drafts.len(), &demoted);
    let submitted = review_submit(
        runner,
        repo,
        number,
        &sha,
        ReviewEvent::Comment,
        &body,
        &drafts,
    )?;
    Ok(ReviewPostOutcome {
        posted: true,
        review_id: Some(submitted.id),
        threads: pass_threads(&read_threads()?, run_id, pass),
    })
}

/// Posts the findings of one pass as a single COMMENT review; a replayed pass posts nothing.
pub fn review_post(
    runner: &dyn ProcessRunner,
    repo: &str,
    number: u64,
    findings_path: &Path,
    pass: u32,
    run_id: &str,
) -> Result<ReviewPostOutcome, IntegrationError> {
    let number_text = number.to_string();
    let read_threads = || pr_review_threads(Some(&number_text), Some(repo), true).map_err(upstream);
    review_post_with(
        runner,
        &read_threads,
        repo,
        number,
        findings_path,
        pass,
        run_id,
    )
}

/// Replies on a thread then reopens it, through the supplied GitHub calls.
pub fn thread_reopen_with(
    reply: &dyn Fn(&str, &str) -> Result<(), IntegrationError>,
    resolve: &dyn Fn(&[String], bool) -> Vec<ThreadResult>,
    thread_id: &str,
    body: &str,
) -> Result<ThreadReopenOutcome, IntegrationError> {
    reply(thread_id, body)?;
    let results = resolve(&[thread_id.to_string()], true);
    Ok(ThreadReopenOutcome {
        thread_id: thread_id.to_string(),
        replied: true,
        unresolved: !results.is_empty() && results.iter().all(|result| result.ok),
    })
}

/// Replies on a review thread, then reopens it.
pub fn thread_reopen(thread_id: &str, body: &str) -> Result<ThreadReopenOutcome, IntegrationError> {
    thread_reopen_with(&thread_reply, &thread_resolve, thread_id, body)
}

#[cfg(test)]
#[path = "../tests/unit/review_post_tests.rs"]
mod tests;
