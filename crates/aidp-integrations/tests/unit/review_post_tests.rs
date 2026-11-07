use super::*;
use crate::models::enums::diff_side::DiffSide;
use aidp_core::models::process::ScriptedOutput;
use aidp_core::process_runner::{ScriptedProcessRunner, SpawnedChild};
use std::cell::{Cell, RefCell};
use std::process::Output;
use std::sync::Mutex;

const FILES: &str = include_str!("fixtures/review_post/pr_files.json");
const FINDINGS: &str = include_str!("fixtures/review_post/findings.json");
const THREADS: &str = include_str!("fixtures/review_post/threads_with_markers.json");
const REVIEWS_WITH_MARKER: &str = include_str!("fixtures/review_post/reviews_with_marker.json");

const REPO: &str = "o/r";
const NUMBER: u64 = 7;
const RUN_ID: &str = "run-7";
const PASS: u32 = 1;
const NO_THREADS: &str = r#"{"threads":[]}"#;
const NO_REVIEWS: &str = "[]";
const HEAD_REPLY: &str = r#"{"head":{"sha":"abc123"}}"#;
const SUBMIT_REPLY: &str = r#"{"id":99,"state":"COMMENTED"}"#;
const INPUT_FLAG: &str = "--input";
const FILES_NEEDLE: &str = "/files?";
const REVIEWS_SUFFIX: &str = "/reviews";
const REVIEW_ID: u64 = 99;
const MARKER_TEXT: &str = "<!-- aidp:finding id=F-1 run=run-7 pass=2 -->";
const NOT_JSON: &str = "not json";
const NOT_SCRIPTED: &str = "spawn is not scripted";

struct RouterRunner {
    reviews: &'static str,
    calls: Mutex<Vec<Vec<String>>>,
    submitted: Mutex<Vec<Value>>,
}

impl RouterRunner {
    fn new(reviews: &'static str) -> Self {
        Self {
            reviews,
            calls: Mutex::new(Vec::new()),
            submitted: Mutex::new(Vec::new()),
        }
    }

    fn submitted(&self) -> Vec<Value> {
        self.submitted.lock().unwrap().clone()
    }

    fn call_count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }

    fn reply_for(&self, args: &[String]) -> String {
        if let Some(position) = args.iter().position(|arg| arg == INPUT_FLAG) {
            let body = std::fs::read_to_string(&args[position + 1]).unwrap();
            self.submitted
                .lock()
                .unwrap()
                .push(serde_json::from_str(&body).unwrap());
            return SUBMIT_REPLY.to_string();
        }
        let path = args.last().unwrap();
        if path.contains(FILES_NEEDLE) {
            FILES.to_string()
        } else if path.ends_with(REVIEWS_SUFFIX) {
            self.reviews.to_string()
        } else {
            HEAD_REPLY.to_string()
        }
    }
}

impl ProcessRunner for RouterRunner {
    fn output(&self, spec: &ProcessSpec) -> std::io::Result<Output> {
        self.calls.lock().unwrap().push(spec.args.clone());
        let reply = self.reply_for(&spec.args);
        ScriptedProcessRunner::new()
            .with_output(ScriptedOutput {
                status_code: 0,
                stdout: reply.into_bytes(),
                stderr: Vec::new(),
            })
            .output(spec)
    }

    fn spawn(&self, _spec: &ProcessSpec) -> std::io::Result<Box<dyn SpawnedChild>> {
        Err(std::io::Error::other(NOT_SCRIPTED))
    }
}

fn findings_file(content: &str) -> tempfile::NamedTempFile {
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), content).unwrap();
    file
}

fn reads(replies: Vec<&'static str>) -> impl Fn() -> Result<Value, IntegrationError> {
    let index = Cell::new(0);
    move || {
        let reply = replies[index.get().min(replies.len() - 1)];
        index.set(index.get() + 1);
        Ok(serde_json::from_str(reply).unwrap())
    }
}

#[test]
fn review_post_marker_round_trips() {
    let marker = FindingMarker {
        id: "F-1".into(),
        run: RUN_ID.into(),
        pass: 2,
    };
    assert_eq!(marker.render(), MARKER_TEXT);
    assert_eq!(FindingMarker::parse(&marker.render()), Some(marker));
    assert_eq!(FindingMarker::parse("no marker here"), None);
}

#[test]
fn review_post_splits_inline_and_out_of_diff() {
    let runner = RouterRunner::new(NO_REVIEWS);
    let file = findings_file(FINDINGS);
    let read = reads(vec![NO_THREADS, THREADS]);
    let outcome =
        review_post_with(&runner, &read, REPO, NUMBER, file.path(), PASS, RUN_ID).unwrap();
    assert!(outcome.posted);
    assert_eq!(outcome.review_id, Some(REVIEW_ID));
    assert_eq!(outcome.threads.len(), 2);
    let submitted = runner.submitted();
    assert_eq!(submitted.len(), 1);
    let review = &submitted[0];
    assert_eq!(review["event"], "COMMENT");
    assert_eq!(review["commit_id"], "abc123");
    let comments = review["comments"].as_array().unwrap();
    assert_eq!(comments.len(), 2);
    assert_eq!(comments[0]["path"], "src/a.rs");
    assert_eq!(comments[0]["line"], 11);
    assert_eq!(comments[0]["side"], "LEFT");
    assert!(comments[0]["body"]
        .as_str()
        .unwrap()
        .starts_with("<!-- aidp:finding id=F-1 run=run-7 pass=1 -->"));
    assert_eq!(comments[1]["path"], "src/new_name.rs");
    assert_eq!(comments[1]["line"], 2);
    assert_eq!(comments[1]["side"], "RIGHT");
    let body = review["body"].as_str().unwrap();
    assert!(body.contains("`src/a.rs:99`"));
    assert!(body.contains("id=F-3 run=run-7 pass=1"));
    assert!(!body.contains("id=F-1"));
}

#[test]
fn review_post_replay_posts_nothing() {
    let runner = RouterRunner::new(REVIEWS_WITH_MARKER);
    let file = findings_file(FINDINGS);
    let read = reads(vec![THREADS]);
    let outcome =
        review_post_with(&runner, &read, REPO, NUMBER, file.path(), PASS, RUN_ID).unwrap();
    assert!(!outcome.posted);
    assert_eq!(outcome.review_id, None);
    let thread_ids: Vec<&str> = outcome
        .threads
        .iter()
        .map(|thread| thread.thread_id.as_str())
        .collect();
    assert_eq!(thread_ids, vec!["T-1", "T-2"]);
    assert!(runner.submitted().is_empty());
    assert_eq!(runner.call_count(), 1);
}

#[test]
fn review_post_new_pass_posts_again() {
    let runner = RouterRunner::new(REVIEWS_WITH_MARKER);
    let file = findings_file(FINDINGS);
    let read = reads(vec![THREADS]);
    let outcome = review_post_with(&runner, &read, REPO, NUMBER, file.path(), 2, RUN_ID).unwrap();
    assert!(outcome.posted);
    assert_eq!(runner.submitted().len(), 1);
}

#[test]
fn review_post_rejects_invalid_findings_file() {
    let runner = RouterRunner::new(NO_REVIEWS);
    let file = findings_file(NOT_JSON);
    let read = reads(vec![NO_THREADS]);
    let error =
        review_post_with(&runner, &read, REPO, NUMBER, file.path(), PASS, RUN_ID).unwrap_err();
    assert!(matches!(error, IntegrationError::Conflict(_)));
}

#[test]
fn review_post_anchor_resolves_renamed_file_and_side() {
    let files = crate::pr_detail::parse_files_page(FILES, 1, 100)
        .unwrap()
        .files;
    let old_name = FindingLocation {
        path: "src/old_name.rs".into(),
        line: 2,
        side: DiffSide::Right,
        start_line: None,
    };
    assert_eq!(
        anchor_in_diff(&files, &old_name).unwrap().path,
        "src/new_name.rs"
    );
    let deleted = FindingLocation {
        path: "src/a.rs".into(),
        line: 11,
        side: DiffSide::Left,
        start_line: None,
    };
    assert!(anchor_in_diff(&files, &deleted).is_some());
    let far_away = FindingLocation {
        side: DiffSide::Right,
        line: 99,
        ..deleted
    };
    assert!(anchor_in_diff(&files, &far_away).is_none());
}

#[test]
fn review_post_thread_reopen_replies_then_unresolves() {
    let order: RefCell<Vec<String>> = RefCell::new(Vec::new());
    let reply = |id: &str, body: &str| {
        order.borrow_mut().push(format!("reply {id} {body}"));
        Ok(())
    };
    let resolve = |ids: &[String], unresolve: bool| {
        order
            .borrow_mut()
            .push(format!("resolve {} {unresolve}", ids[0]));
        vec![ThreadResult {
            id: ids[0].clone(),
            ok: true,
            error: None,
        }]
    };
    let outcome = thread_reopen_with(&reply, &resolve, "T-1", "still broken").unwrap();
    assert!(outcome.replied && outcome.unresolved);
    assert_eq!(
        *order.borrow(),
        vec![
            "reply T-1 still broken".to_string(),
            "resolve T-1 true".to_string()
        ]
    );
}

#[test]
fn review_post_thread_reopen_skips_unresolve_when_reply_fails() {
    let order: RefCell<Vec<&str>> = RefCell::new(Vec::new());
    let reply = |_: &str, _: &str| Err(IntegrationError::Upstream("boom".into()));
    let resolve = |_: &[String], _: bool| {
        order.borrow_mut().push("resolve");
        Vec::new()
    };
    assert!(thread_reopen_with(&reply, &resolve, "T-1", "x").is_err());
    assert!(order.borrow().is_empty());
}
