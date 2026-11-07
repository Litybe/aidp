use super::*;
use serde_json::Value;
use std::collections::HashSet;

const CONVERSATION: &str = include_str!("fixtures/review_threads_conversation.json");
const VIEWER: &str = "alexis";
const THREADS_POINTER: &str = "/data/repository/pullRequest/reviewThreads/nodes";
const CONVERSATION_ID: &str = "T_conv";
const OUTDATED_ID: &str = "T_outdated";
const RESOLVED_ID: &str = "T_resolved";
const FIRST_COMMENT_DIFF_HUNK: &str = "@@ -8,3 +8,3 @@ fn a() {";

fn node(index: usize) -> Value {
    let root: Value = serde_json::from_str(CONVERSATION).unwrap();
    root.pointer(THREADS_POINTER).unwrap()[index].clone()
}

fn derive(index: usize, handed: &HashSet<String>) -> ReviewThread {
    derive_review_thread(&node(index), VIEWER, handed)
}

#[test]
fn conversation_last_by_me_is_waiting_on_reviewer() {
    let thread = derive(0, &HashSet::new());
    assert_eq!(thread.comments.len(), 3);
    assert_eq!(thread.last_by, LastBy::Me);
    assert!(thread.waiting_on_reviewer);
    assert_eq!(thread.start_line, Some(8));
    assert!(thread.comments[1].is_mine);
    assert!(!thread.comments[0].is_mine);
    assert_eq!(thread.addressed, Addressed::No);
}

/// The first comment exposes its diff hunk; a comment without one yields None.
#[test]
fn first_comment_exposes_diff_hunk_and_missing_yields_none() {
    let thread = derive(0, &HashSet::new());
    assert_eq!(
        thread.comments[0].diff_hunk.as_deref(),
        Some(FIRST_COMMENT_DIFF_HUNK)
    );
    assert_eq!(thread.comments[1].diff_hunk, None);
}

#[test]
fn outdated_thread_is_directly_addressed() {
    let thread = derive(1, &HashSet::new());
    assert_eq!(thread.addressed, Addressed::Direct);
    assert_eq!(thread.thread_id.as_deref(), Some(OUTDATED_ID));
    assert_eq!(thread.last_by, LastBy::Other);
}

#[test]
fn handed_thread_is_indirectly_addressed() {
    let handed: HashSet<String> = [CONVERSATION_ID.to_string()].into();
    assert_eq!(derive(0, &handed).addressed, Addressed::Indirect);
}

#[test]
fn resolved_thread_is_not_waiting_on_reviewer() {
    let thread = derive(2, &HashSet::new());
    assert_eq!(thread.last_by, LastBy::Me);
    assert!(!thread.waiting_on_reviewer);
    assert_eq!(thread.thread_id.as_deref(), Some(RESOLVED_ID));
}

#[test]
fn legacy_fields_stay_in_the_serialized_thread() {
    let value = serde_json::to_value(derive(0, &HashSet::new())).unwrap();
    assert_eq!(value["thread_id"], CONVERSATION_ID);
    assert_eq!(value["reply_to_id"], "C1");
    assert_eq!(value["is_resolved"], false);
    assert_eq!(value["is_outdated"], false);
    assert_eq!(value["path"], "src/a.rs");
    assert_eq!(value["line"], 10);
    assert_eq!(value["author"], "bob");
    assert_eq!(value["body"], "rename this");
    assert_eq!(value["lastBy"], "me");
    assert_eq!(value["comments"][2]["databaseId"], 3);
    assert!(value.get("commentsTruncated").is_none());
}

#[test]
fn more_than_fifty_comments_set_the_truncated_flag() {
    let mut raw = node(0);
    raw["comments"]["totalCount"] = Value::from(51);
    let thread = derive_review_thread(&raw, VIEWER, &HashSet::new());
    assert!(thread.comments_truncated);
    assert_eq!(
        serde_json::to_value(thread).unwrap()["commentsTruncated"],
        true
    );
}
