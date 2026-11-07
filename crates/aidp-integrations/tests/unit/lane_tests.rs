use super::*;
use crate::models::enums::lane::Lane;
use crate::models::pr_mark::PrMark;
use crate::models::squad_pr::SquadPr;
use crate::pr::ReviewState;

use crate::squad::test_support::*;

#[test]
fn classify_table_one_primary_lane() {
    let cases: Vec<(&str, SquadPr, Vec<Lane>)> = vec![
        ("my quiet pr", pr_by(ME), vec![Lane::Mine]),
        (
            "my approved green pr is mergeable",
            ready_to_merge_pr(),
            vec![Lane::NeedsAction, Lane::Mine],
        ),
        (
            "my pr with red checks",
            red_checks_pr(),
            vec![Lane::NeedsAction, Lane::Mine],
        ),
        (
            "my pr with a conflict",
            conflicting_pr(),
            vec![Lane::NeedsAction, Lane::Mine],
        ),
        (
            "my pr with changes requested",
            changes_requested_on_mine(),
            vec![Lane::NeedsAction, Lane::Mine],
        ),
        ("my draft stays mine", draft_of(ME), vec![Lane::Mine]),
        (
            "colleague pr never reviewed",
            pr_by("bob"),
            vec![Lane::FirstReview],
        ),
        (
            "colleague pr requesting me",
            requested_pr(),
            vec![Lane::FirstReview],
        ),
        (
            "bot pr is a first review",
            pr_by("dependabot[bot]"),
            vec![Lane::FirstReview],
        ),
        ("colleague draft", draft_of("bob"), vec![Lane::Drafts]),
        (
            "stale approval",
            with_reviews(
                pr_by("bob"),
                vec![review(ME, ReviewState::Approved, OLD_HEAD, "t1")],
            ),
            vec![Lane::ReReview],
        ),
        (
            "stale beats draft",
            with_reviews(
                draft_of("bob"),
                vec![review(ME, ReviewState::Approved, OLD_HEAD, "t1")],
            ),
            vec![Lane::ReReview],
        ),
        (
            "dismissed review reopens first review",
            with_reviews(
                pr_by("bob"),
                vec![review(ME, ReviewState::Dismissed, HEAD, "t1")],
            ),
            vec![Lane::FirstReview],
        ),
        (
            "commented with open threads",
            open_discussion_pr(),
            vec![Lane::Discussion],
        ),
        (
            "commented without open threads",
            with_reviews(
                pr_by("bob"),
                vec![review(ME, ReviewState::Commented, HEAD, "t1")],
            ),
            vec![Lane::Reviewed],
        ),
        (
            "approved on head",
            with_reviews(
                pr_by("bob"),
                vec![review(ME, ReviewState::Approved, HEAD, "t1")],
            ),
            vec![Lane::Reviewed],
        ),
        (
            "changes requested on head",
            with_reviews(
                pr_by("bob"),
                vec![review(ME, ReviewState::ChangesRequested, HEAD, "t1")],
            ),
            vec![Lane::Reviewed],
        ),
    ];
    for (name, pr, expected) in cases {
        assert_eq!(classify(&pr, ME, None), expected, "{name}");
    }
}

#[test]
fn classify_gives_a_colleague_pr_exactly_one_lane_and_never_mine() {
    let colleague_prs = [
        pr_by("bob"),
        requested_pr(),
        draft_of("bob"),
        open_discussion_pr(),
        with_reviews(
            pr_by("bob"),
            vec![review(ME, ReviewState::Approved, OLD_HEAD, "t1")],
        ),
    ];
    for pr in colleague_prs {
        let lanes = classify(&pr, ME, None);
        assert_eq!(lanes.len(), 1);
        assert!(!lanes.contains(&Lane::Mine));
    }
}

#[test]
fn classify_puts_every_own_pr_in_mine() {
    let own_prs = [
        pr_by(ME),
        ready_to_merge_pr(),
        red_checks_pr(),
        conflicting_pr(),
        draft_of(ME),
    ];
    for pr in own_prs {
        assert!(classify(&pr, ME, None).contains(&Lane::Mine));
    }
}

#[test]
fn ignored_mark_removes_a_colleague_pr_from_every_lane() {
    let ignored = PrMark {
        ignored: true,
        ..PrMark::default()
    };
    assert!(classify(&pr_by("bob"), ME, Some(&ignored)).is_empty());
    assert!(classify(&draft_of("bob"), ME, Some(&ignored)).is_empty());
}

#[test]
fn ignored_mark_does_not_hide_my_own_pr() {
    let ignored = PrMark {
        ignored: true,
        ..PrMark::default()
    };
    assert_eq!(classify(&pr_by(ME), ME, Some(&ignored)), vec![Lane::Mine]);
}

#[test]
fn seen_mark_is_unseen_once_the_head_moves() {
    let mark = PrMark {
        seen_sha: Some("sha-x".to_string()),
        seen_at: Some(NOW.to_string()),
        ignored: false,
        ..PrMark::default()
    };
    assert!(mark.is_seen("sha-x"));
    assert!(!mark.is_seen("sha-y"));
    assert!(!PrMark::default().is_seen("sha-x"));
}

#[test]
fn mark_key_joins_repo_and_number() {
    assert_eq!(mark_key("acme/svc", 7), "acme/svc#7");
}
