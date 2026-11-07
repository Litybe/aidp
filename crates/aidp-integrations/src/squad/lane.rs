//! Lane classification of squad pull requests.

use crate::models::enums::lane::Lane;
use crate::models::enums::my_review_state::MyReviewState;
use crate::models::pr_mark::PrMark;
use crate::models::squad_pr::SquadPr;
use crate::pr::{ChecksVerdict, Mergeable, ReviewDecision, StatusContextState};
use crate::squad::review_state::my_review_state;

/// Key of a pull request in the marks file: `owner/repo#number`.
pub fn mark_key(repo: &str, number: u64) -> String {
    format!("{repo}#{number}")
}

/// Checks verdict of a status rollup state.
pub fn checks_verdict_of(rollup_state: Option<StatusContextState>) -> ChecksVerdict {
    match rollup_state {
        Some(StatusContextState::Failure | StatusContextState::Error) => ChecksVerdict::Failed,
        Some(StatusContextState::Pending | StatusContextState::Expected) => ChecksVerdict::Pending,
        Some(StatusContextState::Success) => ChecksVerdict::Passed,
        _ => ChecksVerdict::None,
    }
}

fn is_changes_requested(pr: &SquadPr) -> bool {
    pr.review_decision == Some(ReviewDecision::ChangesRequested)
}

fn is_ready_to_merge(pr: &SquadPr) -> bool {
    !pr.is_draft
        && pr.review_decision == Some(ReviewDecision::Approved)
        && checks_verdict_of(pr.checks_state) == ChecksVerdict::Passed
        && pr.mergeable == Mergeable::Mergeable
        && pr.unresolved_threads == 0
}

/// My own pull request needs me: changes requested, red checks, conflict or ready to merge.
fn needs_action(pr: &SquadPr) -> bool {
    is_changes_requested(pr)
        || checks_verdict_of(pr.checks_state) == ChecksVerdict::Failed
        || pr.mergeable == Mergeable::Conflicting
        || is_ready_to_merge(pr)
}

fn is_requested_from(pr: &SquadPr, me: &str) -> bool {
    pr.review_requests
        .iter()
        .any(|requested| requested.eq_ignore_ascii_case(me))
}

fn lane_of_other(pr: &SquadPr, me: &str) -> Lane {
    let state = my_review_state(pr, me);
    if state == MyReviewState::Stale {
        return Lane::ReReview;
    }
    let never_reviewed = matches!(state, MyReviewState::None | MyReviewState::Dismissed);
    if !pr.is_draft && (is_requested_from(pr, me) || never_reviewed) {
        return Lane::FirstReview;
    }
    if state == MyReviewState::Commented && pr.unresolved_threads > 0 {
        return Lane::Discussion;
    }
    if pr.is_draft {
        return Lane::Drafts;
    }
    Lane::Reviewed
}

/// Lanes of `pr`: one primary lane, plus `Mine` for my own pull requests.
/// An ignored mark removes someone else's pull request from every lane.
pub fn classify(pr: &SquadPr, me: &str, mark: Option<&PrMark>) -> Vec<Lane> {
    if pr.author.eq_ignore_ascii_case(me) {
        return if needs_action(pr) {
            vec![Lane::NeedsAction, Lane::Mine]
        } else {
            vec![Lane::Mine]
        };
    }
    if mark.is_some_and(|mark| mark.ignored) {
        return Vec::new();
    }
    vec![lane_of_other(pr, me)]
}

#[cfg(test)]
#[path = "../../tests/unit/lane_tests.rs"]
mod tests;
