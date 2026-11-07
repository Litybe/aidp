//! GitHub write calls for a PR page: comments, single-request reviews and failed-job reruns.

use crate::models::body_payload::BodyPayload;
use crate::models::comment_write_outcome::CommentWriteOutcome;
use crate::models::constants::process::GH_PROGRAM;
use crate::models::enums::comment_kind::CommentKind;
use crate::models::enums::review_event::ReviewEvent;
use crate::models::inline_comment_payload::InlineCommentPayload;
use crate::models::review_comment_payload::ReviewCommentPayload;
use crate::models::review_draft::ReviewDraft;
use crate::models::review_payload::ReviewPayload;
use crate::models::review_submit_outcome::ReviewSubmitOutcome;
use crate::IntegrationError;
use aidp_core::models::process::ProcessSpec;
use aidp_core::process_runner::ProcessRunner;
use serde::Serialize;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

const GH_API: &str = "api";
const METHOD_FLAG: &str = "-X";
const INPUT_FLAG: &str = "--input";
const POST: &str = "POST";
const PATCH: &str = "PATCH";
const DELETE: &str = "DELETE";
const TEMP_PREFIX: &str = "aidp-gh-body-";
const TEMP_SUFFIX: &str = ".json";
const BAD_REPLY: &str = "unexpected reply from gh";

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Field names read from GitHub replies.
mod wire {
    pub const ID: &str = "id";
    pub const HTML_URL: &str = "html_url";
    pub const STATE: &str = "state";
    pub const USER: &str = "user";
    pub const LOGIN: &str = "login";
}

fn upstream(error: impl ToString) -> IntegrationError {
    IntegrationError::Upstream(error.to_string())
}

fn bad_reply() -> IntegrationError {
    IntegrationError::Upstream(BAD_REPLY.to_string())
}

pub(crate) fn run_gh(
    runner: &dyn ProcessRunner,
    args: Vec<String>,
) -> Result<String, IntegrationError> {
    let spec = ProcessSpec::new(GH_PROGRAM).args(args);
    let output = runner.output(&spec)?;
    if !output.status.success() {
        return Err(upstream(String::from_utf8_lossy(&output.stderr).trim()));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

pub(crate) fn strings(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|part| part.to_string()).collect()
}

fn parse(text: &str) -> Result<Value, IntegrationError> {
    serde_json::from_str(text).map_err(upstream)
}

/// Request body written to a temp file for `gh api --input`, removed on drop.
struct BodyFile(PathBuf);

impl BodyFile {
    fn write(payload: &impl Serialize) -> Result<Self, IntegrationError> {
        let sequence = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let name = format!(
            "{TEMP_PREFIX}{}-{sequence}{TEMP_SUFFIX}",
            std::process::id()
        );
        let path = std::env::temp_dir().join(name);
        let text = serde_json::to_string(payload).map_err(upstream)?;
        std::fs::write(&path, text)?;
        Ok(Self(path))
    }

    fn path(&self) -> String {
        self.0.to_string_lossy().into_owned()
    }
}

impl Drop for BodyFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

pub(crate) fn send(
    runner: &dyn ProcessRunner,
    method: &str,
    path: String,
    payload: Option<&impl Serialize>,
) -> Result<String, IntegrationError> {
    let mut args = strings(&[GH_API, METHOD_FLAG, method]);
    args.push(path);
    let file = match payload {
        Some(body) => Some(BodyFile::write(body)?),
        None => None,
    };
    if let Some(file) = &file {
        args.extend([INPUT_FLAG.to_string(), file.path()]);
    }
    run_gh(runner, args)
}

fn text_of(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_string)
}

fn comment_outcome(text: &str) -> Result<CommentWriteOutcome, IntegrationError> {
    let value = parse(text)?;
    let id = value
        .get(wire::ID)
        .and_then(Value::as_u64)
        .ok_or_else(bad_reply)?;
    Ok(CommentWriteOutcome {
        id,
        url: text_of(&value, wire::HTML_URL),
    })
}

fn global_path(repo: &str, number: u64) -> String {
    format!("repos/{repo}/issues/{number}/comments")
}

fn comment_path(repo: &str, kind: CommentKind, id: u64) -> String {
    match kind {
        CommentKind::Conversation => format!("repos/{repo}/issues/comments/{id}"),
        CommentKind::Review => format!("repos/{repo}/pulls/comments/{id}"),
    }
}

/// Posts a conversation comment on the PR.
pub fn comment_create(
    runner: &dyn ProcessRunner,
    repo: &str,
    number: u64,
    body: &str,
) -> Result<CommentWriteOutcome, IntegrationError> {
    let payload = BodyPayload { body };
    comment_outcome(&send(
        runner,
        POST,
        global_path(repo, number),
        Some(&payload),
    )?)
}

/// Replaces the text of a conversation or inline comment.
pub fn comment_edit(
    runner: &dyn ProcessRunner,
    repo: &str,
    kind: CommentKind,
    id: u64,
    body: &str,
) -> Result<CommentWriteOutcome, IntegrationError> {
    let payload = BodyPayload { body };
    comment_outcome(&send(
        runner,
        PATCH,
        comment_path(repo, kind, id),
        Some(&payload),
    )?)
}

/// Deletes a conversation or inline comment.
pub fn comment_delete(
    runner: &dyn ProcessRunner,
    repo: &str,
    kind: CommentKind,
    id: u64,
) -> Result<(), IntegrationError> {
    send(
        runner,
        DELETE,
        comment_path(repo, kind, id),
        None::<&BodyPayload>,
    )
    .map(|_| ())
}

fn anchor(draft: &ReviewDraft) -> ReviewCommentPayload<'_> {
    ReviewCommentPayload {
        path: &draft.path,
        line: draft.line,
        side: draft.side,
        start_line: draft.start_line,
        start_side: draft.start_side,
        body: &draft.body,
    }
}

/// Posts one inline comment pinned to the PR head commit, outside any review.
pub fn inline_comment_create(
    runner: &dyn ProcessRunner,
    repo: &str,
    number: u64,
    head_sha: &str,
    draft: &ReviewDraft,
) -> Result<CommentWriteOutcome, IntegrationError> {
    let payload = InlineCommentPayload {
        commit_id: head_sha,
        comment: anchor(draft),
    };
    let path = format!("repos/{repo}/pulls/{number}/comments");
    comment_outcome(&send(runner, POST, path, Some(&payload))?)
}

/// Builds the review request: verdict, summary and every draft comment.
pub fn review_payload<'a>(
    event: ReviewEvent,
    body: &'a str,
    head_sha: &'a str,
    drafts: &'a [ReviewDraft],
) -> ReviewPayload<'a> {
    ReviewPayload {
        event,
        body,
        commit_id: head_sha,
        comments: drafts.iter().map(anchor).collect(),
    }
}

/// Submits the review and all its inline comments in one request, never leaving a pending review.
pub fn review_submit(
    runner: &dyn ProcessRunner,
    repo: &str,
    number: u64,
    head_sha: &str,
    event: ReviewEvent,
    body: &str,
    drafts: &[ReviewDraft],
) -> Result<ReviewSubmitOutcome, IntegrationError> {
    let payload = review_payload(event, body, head_sha, drafts);
    let path = format!("repos/{repo}/pulls/{number}/reviews");
    let value = parse(&send(runner, POST, path, Some(&payload))?)?;
    let id = value
        .get(wire::ID)
        .and_then(Value::as_u64)
        .ok_or_else(bad_reply)?;
    Ok(ReviewSubmitOutcome {
        id,
        state: text_of(&value, wire::STATE).unwrap_or_default(),
        url: text_of(&value, wire::HTML_URL),
        comment_count: drafts.len(),
    })
}

/// Login of the account that wrote the comment, so callers can refuse foreign edits.
pub fn comment_author(
    runner: &dyn ProcessRunner,
    repo: &str,
    kind: CommentKind,
    id: u64,
) -> Result<String, IntegrationError> {
    let args = vec![GH_API.to_string(), comment_path(repo, kind, id)];
    let value = parse(&run_gh(runner, args)?)?;
    value
        .get(wire::USER)
        .and_then(|user| text_of(user, wire::LOGIN))
        .ok_or_else(bad_reply)
}

#[cfg(test)]
#[path = "tests/units/pr_writes_tests.rs"]
mod pr_writes_tests;
