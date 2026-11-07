use super::*;

const CONVERSATION: &str = include_str!("fixtures/issue_comments.json");
const REVIEWS: &str = include_str!("fixtures/review_bodies.json");

fn parse(bots: &[&str]) -> Vec<GlobalComment> {
    let configured: Vec<String> = bots.iter().map(|bot| bot.to_string()).collect();
    parse_comments(CONVERSATION, REVIEWS, &configured).unwrap()
}

fn by_id(comments: &[GlobalComment], id: u64) -> &GlobalComment {
    comments.iter().find(|comment| comment.id == id).unwrap()
}

#[test]
fn bot_suffix_marks_bot() {
    let comments = parse(&[]);
    assert!(by_id(&comments, 12).bot);
    assert!(!by_id(&comments, 13).bot);
}

#[test]
fn configured_author_marks_bot() {
    assert!(by_id(&parse(&["ci-helper"]), 13).bot);
}

#[test]
fn summary_marker_comment_is_excluded() {
    assert!(parse(&[]).iter().all(|comment| comment.id != 14));
}

#[test]
fn empty_review_body_is_skipped() {
    let comments = parse(&[]);
    assert!(comments.iter().all(|comment| comment.id != 22));
    assert_eq!(by_id(&comments, 21).kind, CommentKind::Review);
    assert_eq!(by_id(&comments, 21).created_at, "2026-10-02T10:00:00Z");
}

#[test]
fn point_ids_follow_comment_id_and_index() {
    let comments = parse(&[]);
    let ids: Vec<_> = by_id(&comments, 11)
        .points
        .iter()
        .map(|point| point.id.clone().unwrap())
        .collect();
    assert_eq!(ids, ["11#0", "11#1"]);
}
