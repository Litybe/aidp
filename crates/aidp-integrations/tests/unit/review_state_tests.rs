use super::*;
use crate::models::enums::my_review_state::MyReviewState;
use crate::models::squad_pr::SquadPr;
use crate::pr::ReviewState;

use crate::squad::test_support::*;

#[test]
fn my_review_state_table() {
    let cases: Vec<(&str, SquadPr, MyReviewState)> = vec![
        ("no review", pr_by("bob"), MyReviewState::None),
        (
            "approved on head",
            with_reviews(
                pr_by("bob"),
                vec![review(ME, ReviewState::Approved, HEAD, "t1")],
            ),
            MyReviewState::Approved,
        ),
        (
            "approved on old sha then new head",
            with_reviews(
                pr_by("bob"),
                vec![review(ME, ReviewState::Approved, OLD_HEAD, "t1")],
            ),
            MyReviewState::Stale,
        ),
        (
            "dismissed",
            with_reviews(
                pr_by("bob"),
                vec![review(ME, ReviewState::Dismissed, HEAD, "t1")],
            ),
            MyReviewState::Dismissed,
        ),
        (
            "changes requested on head",
            with_reviews(
                pr_by("bob"),
                vec![review(ME, ReviewState::ChangesRequested, HEAD, "t1")],
            ),
            MyReviewState::ChangesRequested,
        ),
        (
            "commented only",
            with_reviews(
                pr_by("bob"),
                vec![review(ME, ReviewState::Commented, HEAD, "t1")],
            ),
            MyReviewState::Commented,
        ),
        (
            "latest opinionated review wins",
            with_reviews(
                pr_by("bob"),
                vec![
                    review(ME, ReviewState::ChangesRequested, HEAD, "t1"),
                    review(ME, ReviewState::Approved, HEAD, "t2"),
                ],
            ),
            MyReviewState::Approved,
        ),
        (
            "someone else's review is not mine",
            with_reviews(
                pr_by("bob"),
                vec![review("carol", ReviewState::Approved, HEAD, "t1")],
            ),
            MyReviewState::None,
        ),
        (
            "login comparison ignores case",
            with_reviews(
                pr_by("bob"),
                vec![review("ALICE", ReviewState::Approved, HEAD, "t1")],
            ),
            MyReviewState::Approved,
        ),
    ];
    for (name, pr, expected) in cases {
        assert_eq!(my_review_state(&pr, ME), expected, "{name}");
    }
}
