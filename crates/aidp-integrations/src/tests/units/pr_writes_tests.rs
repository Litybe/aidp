use super::*;
use crate::models::enums::diff_side::DiffSide;
use aidp_core::models::process::ScriptedOutput;
use aidp_core::process_runner::{ScriptedProcessRunner, SpawnedChild};
use std::collections::VecDeque;
use std::sync::Mutex;

const REVIEW_REPLY: &str = include_str!("fixtures/pr_writes/review_submitted.json");
const COMMENT: &str = include_str!("fixtures/pr_writes/comment.json");

const REPO: &str = "o/r";
const HEAD: &str = "abc123";

struct RecordingRunner {
    replies: Mutex<VecDeque<String>>,
    calls: Mutex<Vec<Vec<String>>>,
}

impl RecordingRunner {
    fn new(replies: &[&str]) -> Self {
        Self {
            replies: Mutex::new(replies.iter().map(|reply| reply.to_string()).collect()),
            calls: Mutex::new(Vec::new()),
        }
    }

    fn calls(&self) -> Vec<Vec<String>> {
        self.calls.lock().unwrap().clone()
    }
}

impl ProcessRunner for RecordingRunner {
    fn output(&self, spec: &ProcessSpec) -> std::io::Result<std::process::Output> {
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

fn args(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|part| part.to_string()).collect()
}

fn draft(path: &str, line: u32, body: &str) -> ReviewDraft {
    ReviewDraft {
        id: format!("d-{line}"),
        path: path.to_string(),
        line,
        side: DiffSide::Right,
        start_line: None,
        start_side: None,
        body: body.to_string(),
    }
}

#[test]
fn review_with_two_drafts_is_one_gh_call() {
    let runner = RecordingRunner::new(&[REVIEW_REPLY]);
    let drafts = vec![draft("a.rs", 3, "first"), draft("b.rs", 9, "second")];
    let outcome = review_submit(
        &runner,
        REPO,
        7,
        HEAD,
        ReviewEvent::RequestChanges,
        "summary",
        &drafts,
    )
    .unwrap();
    let calls = runner.calls();
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0][..4],
        args(&["api", "-X", "POST", "repos/o/r/pulls/7/reviews"])[..]
    );
    assert_eq!(calls[0][4], "--input");
    assert_eq!(outcome.id, 4242);
    assert_eq!(outcome.state, "CHANGES_REQUESTED");
    assert_eq!(outcome.comment_count, 2);
}

#[test]
fn review_payload_carries_event_commit_and_every_comment() {
    let mut ranged = draft("b.rs", 9, "second");
    ranged.start_line = Some(5);
    ranged.start_side = Some(DiffSide::Right);
    let drafts = vec![draft("a.rs", 3, "first"), ranged];
    let payload = review_payload(ReviewEvent::RequestChanges, "summary", HEAD, &drafts);
    let value = serde_json::to_value(&payload).unwrap();
    assert_eq!(value["event"], "REQUEST_CHANGES");
    assert_eq!(value["commit_id"], HEAD);
    assert_eq!(value["body"], "summary");
    let comments = value["comments"].as_array().unwrap();
    assert_eq!(comments.len(), 2);
    assert_eq!(comments[0]["side"], "RIGHT");
    assert!(comments[0].get("start_line").is_none());
    assert_eq!(comments[1]["start_line"], 5);
    assert_eq!(comments[1]["start_side"], "RIGHT");
}

#[test]
fn review_events_serialize_to_github_names() {
    let events = [
        ReviewEvent::Approve,
        ReviewEvent::RequestChanges,
        ReviewEvent::Comment,
    ];
    let names: Vec<String> = events
        .iter()
        .map(|event| {
            serde_json::to_value(event)
                .unwrap()
                .as_str()
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(names, ["APPROVE", "REQUEST_CHANGES", "COMMENT"]);
}

#[test]
fn global_comment_paths() {
    let runner = RecordingRunner::new(&[COMMENT, COMMENT, ""]);
    let created = comment_create(&runner, REPO, 7, "hi").unwrap();
    comment_edit(&runner, REPO, CommentKind::Conversation, 55, "edited").unwrap();
    comment_delete(&runner, REPO, CommentKind::Conversation, 55).unwrap();
    let calls = runner.calls();
    assert_eq!(
        calls[0][..4],
        args(&["api", "-X", "POST", "repos/o/r/issues/7/comments"])[..]
    );
    assert_eq!(
        calls[1][..4],
        args(&["api", "-X", "PATCH", "repos/o/r/issues/comments/55"])[..]
    );
    assert_eq!(
        calls[2],
        args(&["api", "-X", "DELETE", "repos/o/r/issues/comments/55"])
    );
    assert_eq!(created.id, 55);
}

#[test]
fn review_comment_edit_delete_and_create_paths() {
    let runner = RecordingRunner::new(&[COMMENT, COMMENT, ""]);
    inline_comment_create(&runner, REPO, 7, HEAD, &draft("a.rs", 3, "x")).unwrap();
    comment_edit(&runner, REPO, CommentKind::Review, 55, "edited").unwrap();
    comment_delete(&runner, REPO, CommentKind::Review, 55).unwrap();
    let calls = runner.calls();
    assert_eq!(calls[0][3], "repos/o/r/pulls/7/comments");
    assert_eq!(calls[1][3], "repos/o/r/pulls/comments/55");
    assert_eq!(
        calls[2],
        args(&["api", "-X", "DELETE", "repos/o/r/pulls/comments/55"])
    );
}

#[test]
fn inline_payload_flattens_anchor_beside_commit() {
    let owned = draft("a.rs", 3, "x");
    let payload = InlineCommentPayload {
        commit_id: HEAD,
        comment: anchor(&owned),
    };
    let value = serde_json::to_value(&payload).unwrap();
    assert_eq!(value["commit_id"], HEAD);
    assert_eq!(value["path"], "a.rs");
    assert_eq!(value["line"], 3);
}

#[test]
fn comment_author_reads_user_login() {
    let runner = RecordingRunner::new(&[COMMENT]);
    let login = comment_author(&runner, REPO, CommentKind::Conversation, 55).unwrap();
    assert_eq!(login, "alice");
    assert_eq!(
        runner.calls()[0],
        args(&["api", "repos/o/r/issues/comments/55"])
    );
}
