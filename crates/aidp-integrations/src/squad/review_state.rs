//! My review state of a pull request, derived from my reviews and the head commit.

use crate::models::enums::my_review_state::MyReviewState;
use crate::models::squad_pr::SquadPr;
use crate::models::squad_review::SquadReview;
use crate::pr::ReviewState;

fn is_opinionated(review: &SquadReview) -> bool {
    matches!(
        review.state,
        ReviewState::Approved | ReviewState::ChangesRequested | ReviewState::Dismissed
    )
}

/// Latest opinionated review of `me`, a dismissed one included.
fn latest_opinionated<'a>(pr: &'a SquadPr, me: &str) -> Option<&'a SquadReview> {
    pr.my_reviews
        .iter()
        .filter(|review| review.author.eq_ignore_ascii_case(me) && is_opinionated(review))
        .max_by(|left, right| left.submitted_at.cmp(&right.submitted_at))
}

fn has_comment_review(pr: &SquadPr, me: &str) -> bool {
    pr.my_reviews.iter().any(|review| {
        review.author.eq_ignore_ascii_case(me) && review.state == ReviewState::Commented
    })
}

/// My state on `pr`: an approval or rejection on an older commit is `Stale`.
pub fn my_review_state(pr: &SquadPr, me: &str) -> MyReviewState {
    let Some(review) = latest_opinionated(pr, me) else {
        return if has_comment_review(pr, me) {
            MyReviewState::Commented
        } else {
            MyReviewState::None
        };
    };
    if review.state == ReviewState::Dismissed {
        return MyReviewState::Dismissed;
    }
    if review.commit_oid.as_deref() != Some(pr.head_sha.as_str()) {
        return MyReviewState::Stale;
    }
    if review.state == ReviewState::Approved {
        MyReviewState::Approved
    } else {
        MyReviewState::ChangesRequested
    }
}

#[cfg(test)]
#[path = "../../tests/unit/review_state_tests.rs"]
mod tests;
